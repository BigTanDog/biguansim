//! 验证生产模块 `platform::shell_guard`：**弹出来的开始菜单/搜索能被收回吗？**
//!
//! 这是给 `Win` 键问题定方案的实测工具（2026-10-22）。它不装键盘钩子，
//! 只跑 `shell_guard::start()`，然后用鼠标点击任务栏左下角把 shell 界面唤出来，
//! 观察前台是否被哨兵按回去。
//!
//! 判定：日志里出现 `SHELL 抢到前台`（searchhost.exe 之类），
//! 紧接着前台又变回 `explorer.exe`，两者时间差就是"从弹出到收回"的耗时。
//!
//! 用法：`cargo run --manifest-path src-tauri/Cargo.toml --example shell_guard_probe`

use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, POINT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowTextW,
    GetWindowThreadProcessId, SetCursorPos, SM_CYSCREEN,
};

use biguansim_lib::platform::shell_guard;

/// 最小 logger：把 `shell_guard` 的日志打到 stdout（它用 `log` crate 记事件）
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        println!("[log] {}", record.args());
    }
    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

/// 会盖在结界之上的 shell 界面所属进程
const SHELL_PROCS: [&str; 4] = [
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchui.exe",
    "shellexperiencehost.exe",
];

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

fn foreground_info() -> Option<(u32, String, String, String)> {
    // SAFETY: 只读前台窗口与其 pid
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(fg, &mut pid);

        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(fg, class_buf.as_mut_ptr(), class_buf.len() as i32);
        let mut title_buf = [0u16; 256];
        let title_len = GetWindowTextW(fg, title_buf.as_mut_ptr(), title_buf.len() as i32);

        Some((
            pid,
            process_name(pid).unwrap_or_else(|| "?".into()),
            String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]),
            String::from_utf16_lossy(&title_buf[..title_len.max(0) as usize]),
        ))
    }
}

/// 点一下任务栏左下角（Win11 上这里是搜索按钮，能可靠唤出 shell 界面）
fn click_start_button() {
    // SAFETY: 只做鼠标定位与一次左键点击，事后还原光标
    unsafe {
        let mut origin = POINT { x: 0, y: 0 };
        GetCursorPos(&mut origin);
        let height = GetSystemMetrics(SM_CYSCREEN);
        SetCursorPos(10, height - 10);
        thread::sleep(Duration::from_millis(60));
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        thread::sleep(Duration::from_millis(40));
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
        thread::sleep(Duration::from_millis(120));
        SetCursorPos(origin.x, origin.y);
    }
}

fn main() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    println!("== shell_guard 验证 ==");
    println!("哨兵运行中：{}", shell_guard::is_running());
    shell_guard::start();
    println!("哨兵已启动（每 20 ms 查一次前台）。\n");

    let begin = Instant::now();
    let shell_hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let hits = shell_hits.clone();

    // 前台监视：shell 界面抢到前台 → 哨兵注入 Esc → 前台回来
    let watcher = {
        thread::spawn(move || {
            let mut last_fg: isize = 0;
            for _ in 0..900 {
                if let Some((pid, name, class, title)) = foreground_info() {
                    let key = pid as isize;
                    if key != last_fg {
                        last_fg = key;
                        let shell = SHELL_PROCS.contains(&name.as_str());
                        if shell {
                            hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        println!(
                            "[{:>5}ms] 前台= {name:<28} class={class} title={title}{}",
                            begin.elapsed().as_millis(),
                            if shell {
                                "   <<< SHELL 抢到前台"
                            } else {
                                ""
                            }
                        );
                    }
                }
                thread::sleep(Duration::from_millis(20));
            }
        })
    };

    // 3 秒后开始点任务栏
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(3));
        for round in 1..=2 {
            println!("\n---------- 点击开始按钮 #{round} ----------");
            click_start_button();
            thread::sleep(Duration::from_secs(5));
        }
    });

    watcher.join().ok();
    println!(
        "\nshell 界面抢到前台共 {} 次；哨兵日志里应有对应条目的『已注入 Esc 收回』。",
        shell_hits.load(std::sync::atomic::Ordering::Relaxed)
    );
    shell_guard::stop();
}
