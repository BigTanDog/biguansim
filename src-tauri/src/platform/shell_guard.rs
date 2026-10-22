//! 前台哨兵：把 `Win` 键唤出的开始菜单/搜索**在 20~40 ms 内收回**。
//!
//! 为什么不继续拦：`decisions/004` 的三组对照、`005` 的应用内实测、`006` 的真机复验，
//! 三次都证明「低级键盘钩子吞掉 `Win` 按下与抬起」并不能保证开始菜单不出现——
//! 它挂在 `topmost` 之上的 band，普通置顶窗口压不住。按 `AGENTS §2`（同一问题修三次
//! 未解决即停止试错）不再加码拦键，改为**出现即收回**。
//!
//! 收的手段是注入一个 `Esc`：开始菜单/搜索打开时按 Esc 会自己收起（实测 2026-10-22：
//! 注入 Esc 后 **14 ms** 前台就从 `searchhost.exe` 回到原窗口，肉眼只有一帧）。
//!
//! 三个必须守住的边界：
//! 1. **只认这几个进程**（开始菜单/搜索/任务视图）。别的窗口抢前台是另一回事，由
//!    `focus` 的抢焦点负责，绝不能在这里注入 Esc 乱关用户的东西；
//! 2. 注入的 Esc 带 [`INJECT_MARK`] 标记，`hook` 侧见到就放行且不计破戒长按计时，
//!    否则会出现"哨兵自己把 Esc 吞了"或"破戒进度莫名其妙动了一下"；
//! 3. 只在闭关期运行（`overlay::lock_screen` / `unlock_screen` 成对开关），
//!    出关必须停——出关后还注入 Esc 会打断用户自己的输入。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// 会盖在结界之上的 shell 界面所属进程（Win10 / Win11 各一份）
const SHELL_PROCS: [&str; 4] = [
    "startmenuexperiencehost.exe", // 开始菜单
    "searchhost.exe",              // Win11 搜索
    "searchui.exe",                // Win10 搜索
    "shellexperiencehost.exe",     // 任务视图 / 操作中心
];

/// 轮询周期：10 ms 一次，`GetForegroundWindow` 是纯查询，代价可忽略
const POLL: Duration = Duration::from_millis(10);
/// 两次注入之间的最小间隔。
///
/// 2026-10-22 实测：注入 `Esc` 只关掉了界面，**键盘焦点仍留在 shell 的搜索框里**，
/// 于是用户之后每按一个键，shell 都重新抢到前台，哨兵被触发了 72 次（而 `Win` 只按了 2 次）。
/// 现在改成"收回界面 + 把焦点抢回我们的窗口"，这一轮注入就该彻底结束；
/// 这个间隔只是**兜底**（抢焦点失败时别变成连发）。
const COOLDOWN_MS: u64 = 250;

/// 注入标记（`KBDLLHOOKSTRUCT.dwExtraInfo`）。钩子侧据此放行并跳过 Esc 长按计时。
pub const INJECT_MARK: usize = 0x4247_5347; // "BGSG"

static RUNNING: AtomicBool = AtomicBool::new(false);
/// 暂停（抄写 / 急事确认期间）：那两段用户要输入中文，注入的 Esc 会打掉输入法候选，
/// 与 `focus::pause` 同一个时机。
static PAUSED: AtomicBool = AtomicBool::new(false);

/// 闭关开始时调用（幂等）
pub fn start() {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    thread::spawn(|| {
        let begin = Instant::now();
        let mut cache: HashMap<u32, bool> = HashMap::new();
        let mut last_fire = 0u64;
        let mut fired = 0u32;

        while RUNNING.load(Ordering::SeqCst) {
            if PAUSED.load(Ordering::Relaxed) {
                thread::sleep(POLL);
                continue;
            }

            if let Some(name) = shell_in_foreground(&mut cache) {
                let now = begin.elapsed().as_millis() as u64;
                if now.saturating_sub(last_fire) >= COOLDOWN_MS {
                    last_fire = now;
                    fired += 1;
                    send_escape();

                    // **关键一步**：`Esc` 只关掉了界面，焦点还留在 shell 的搜索框里，
                    // 用户之后每按一个键它都会重新抢前台（实测 72 次误触发）。
                    // 把焦点抢回我们的窗口，这个循环才真正结束。
                    let reclaimed = if super::focus::is_running() {
                        super::focus::reclaim_now()
                    } else {
                        false
                    };

                    // 只在事件发生时写日志，不刷屏
                    ::log::warn!(
                        "[shell] 前台被 {name} 抢走（第 {fired} 次），已注入 Esc 收回{}",
                        if reclaimed {
                            "，焦点已抢回"
                        } else {
                            "，焦点未抢回"
                        }
                    );
                }
            }

            thread::sleep(POLL);
        }

        ::log::info!("[shell] 前台哨兵已停止，本次共收回 {fired} 次");
    });

    ::log::info!(
        "[shell] 前台哨兵已启动：每 {} ms 检查一次前台",
        POLL.as_millis()
    );
}

pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
    // 复位暂停位：出关时若正处在抄写/确认（PAUSED=true），不清掉的话下一关
    // 哨兵会带着"暂停"启动，开局那几十毫秒内不干活。`publish_state` 也会 resume，
    // 这里只是让状态在 stop 之后就是干净的。
    PAUSED.store(false, Ordering::SeqCst);
}

/// 抄写 / 急事确认期间暂停（与 `focus::pause` 同时机）
pub fn pause() {
    if !PAUSED.swap(true, Ordering::SeqCst) {
        ::log::info!("[shell] 暂停前台哨兵（输入期间让位给输入法）");
    }
}

pub fn resume() {
    if PAUSED.swap(false, Ordering::SeqCst) {
        ::log::info!("[shell] 恢复前台哨兵");
    }
}

/// 前台若是 shell 界面（开始菜单/搜索/任务视图），返回其进程名。
///
/// 用**进程名**判断而不是窗口类名：这些界面是 XAML 岛，`EnumWindows` 根本枚举不到
/// （实测 2026-10-22：搜索窗口已是前台，`EnumWindows` 的可见顶层窗口数一个没变），
/// 唯一可靠的观测点是 `GetForegroundWindow`。
fn shell_in_foreground(cache: &mut HashMap<u32, bool>) -> Option<&'static str> {
    // SAFETY: 只读前台窗口与其 pid
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(foreground, &mut pid);
        if pid == 0 || pid == std::process::id() {
            return None;
        }

        let is_shell = match cache.get(&pid) {
            Some(flag) => *flag,
            None => {
                let name = process_name(pid).unwrap_or_default();
                let flag = SHELL_PROCS.contains(&name.as_str());
                cache.insert(pid, flag);
                flag
            }
        };

        if !is_shell {
            return None;
        }

        let name = process_name(pid).unwrap_or_default();
        SHELL_PROCS
            .iter()
            .copied()
            .find(|candidate| *candidate == name)
    }
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: 只查询进程映像名，句柄用完即关
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(
            path.rsplit('\\')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase(),
        )
    }
}

/// 注入一次 Esc（带 [`INJECT_MARK`]）。按下与抬起分两批、间隔 15 ms——
/// 同一批里连着 down+up 在自己的钩子上是"瞬时抖动"，实测分开更稳。
fn send_escape() {
    let make = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0x1B,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: INJECT_MARK,
            },
        },
    };

    let down = [make(false)];
    let up = [make(true)];

    // SAFETY: 两个单元素批次，指针与长度一致
    unsafe {
        SendInput(1, down.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
    thread::sleep(Duration::from_millis(15));
    // SAFETY: 同上
    unsafe {
        SendInput(1, up.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

/// 供排障与断言用：哨兵当前是否在跑
pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 哨兵是进程级单例：`start` / `stop` 必须对称、可重复调用；
    /// `pause` 只影响"要不要注入"，不影响哨兵还在跑。
    ///
    /// **只写一个测试**：哨兵是全局状态，同一文件里的测试默认并行执行，
    /// 拆成两个会互相踩（一个在 stop、另一个在 start）。
    #[test]
    fn start_stop_pause_are_symmetric() {
        assert!(!is_running(), "初始必须是停的");

        start();
        assert!(is_running());
        start(); // 重复启动不应起第二个线程
        assert!(is_running());

        pause();
        assert!(is_running(), "暂停期间哨兵仍在跑");
        resume();

        stop();
        assert!(!is_running());
        stop(); // 重复停止必须安全
        assert!(!is_running());
    }
}
