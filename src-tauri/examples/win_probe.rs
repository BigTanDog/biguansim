//! `Win` 键诊断探针（临时工具，不参与打包）。
//!
//! 要回答两个问题，它们决定修法完全不同：
//!
//! **Q1：真键盘按 `Win` 时，`WH_KEYBOARD_LL` 到底有没有收到事件？**
//! - 收到了、返回 1、开始菜单仍弹 → 系统有绕开低级钩子的路径，加码拦键没用，
//!   只能"弹出来就按回去"（`decisions/006` 的 B 案，但做得更狠）。
//! - 没收到 → 钩子被系统摘除/没覆盖到，修法是重装策略与线程优先级。
//!
//! **Q2：开始菜单窗口出现后，`WM_CLOSE` / `SW_HIDE` 能不能立刻让它消失？**
//! 这是"按回去"能不能做到"看不见"的前提。
//!
//! 用法（`synth` 可选：先用自己的合成按键做对照，再等真人按键）：
//!
//! ```text
//! cargo run --manifest-path src-tauri/Cargo.toml --example win_probe -- 25
//! cargo run --manifest-path src-tauri/Cargo.toml --example win_probe -- 25 synth
//! ```
//!
//! 安全出口：按 `Ctrl+Alt+Shift+Q`，或到时限自动退出，或 `taskkill /IM win_probe.exe /F`。

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    mouse_event, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, VK_LWIN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetClassNameW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetSystemMetrics, GetWindowTextW, GetWindowThreadProcessId, PostThreadMessageW, SetCursorPos,
    SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, SM_CYSCREEN, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use biguansim_lib::platform::keyfilter::{decide, ModState, Verdict};

/// 输出同时写 stdout 与临时日志文件（方便事后把证据交给别人看）
static LOG_FILE: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();

fn log_to_file(line: &str) {
    if let Some(file) = LOG_FILE.get() {
        if let Ok(mut guard) = file.lock() {
            let _ = writeln!(guard, "{line}");
            let _ = guard.flush();
        }
    }
}

macro_rules! say {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        println!("{line}");
        let _ = std::io::stdout().flush();
        log_to_file(&line);
    }};
}

macro_rules! init_log {
    ($path:expr) => {{
        if let Ok(file) = std::fs::File::create($path) {
            let _ = LOG_FILE.set(std::sync::Mutex::new(file));
        }
    }};
}

const BIT_LWIN: u8 = 1 << 0;
const BIT_RWIN: u8 = 1 << 1;
const BIT_ALT: u8 = 1 << 2;
const BIT_CTRL: u8 = 1 << 3;
const BIT_SHIFT: u8 = 1 << 4;

/// 会顶在结界之上的 shell 界面所属进程（决定"按回去"要打谁）
const TARGETS: [&str; 4] = [
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchui.exe",
    "shellexperiencehost.exe",
];

static MODS: AtomicU8 = AtomicU8::new(0);
static RUNNING: AtomicBool = AtomicBool::new(true);
/// `pass` 模式：放行 `Win` 键，用来验证"合成 Win 单击真的能唤出开始菜单"，
/// 以及 watcher 的关闭动作能不能把它按回去（无需真人按键）。
static PASS_WIN: AtomicBool = AtomicBool::new(false);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static START: AtomicU64 = AtomicU64::new(0);

/// 回调里不许打印（超 `LowLevelHooksTimeout` 会被摘钩子），只写无锁环
const RING_LEN: usize = 512;
static RING: [AtomicU64; RING_LEN] = [const { AtomicU64::new(0) }; RING_LEN];
static RING_HEAD: AtomicUsize = AtomicUsize::new(0);

fn now_ms() -> u64 {
    let start = START.load(Ordering::Relaxed);
    if start == 0 {
        return 0;
    }
    // 用 Instant 需要可比较的基准；这里用单调时钟差的近似（探针不要求精确）
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    elapsed.saturating_sub(start)
}

fn push_event(vk: u16, is_down: bool, swallowed: bool, flags: u32) {
    let seq = RING_HEAD.fetch_add(1, Ordering::Relaxed);
    let packed = ((seq as u64) << 32)
        | (((flags & 0xFF) as u64) << 24)
        | ((swallowed as u64) << 17)
        | ((is_down as u64) << 16)
        | vk as u64;
    RING[seq % RING_LEN].store(packed, Ordering::Release);
}

fn decode(bits: u8) -> ModState {
    ModState {
        lwin: bits & BIT_LWIN != 0,
        rwin: bits & BIT_RWIN != 0,
        alt: bits & BIT_ALT != 0,
        ctrl: bits & BIT_CTRL != 0,
        shift: bits & BIT_SHIFT != 0,
    }
}

fn encode(state: &ModState) -> u8 {
    let mut bits = 0u8;
    if state.lwin {
        bits |= BIT_LWIN;
    }
    if state.rwin {
        bits |= BIT_RWIN;
    }
    if state.alt {
        bits |= BIT_ALT;
    }
    if state.ctrl {
        bits |= BIT_CTRL;
    }
    if state.shift {
        bits |= BIT_SHIFT;
    }
    bits
}

/// 把环里的按键记录搬到 stdout（与钩子回调解耦）
fn spawn_reporter() {
    thread::spawn(|| {
        let mut cursor = 0usize;
        loop {
            let head = RING_HEAD.load(Ordering::Acquire);
            while cursor < head {
                let packed = RING[cursor % RING_LEN].load(Ordering::Acquire);
                if (packed >> 32) as usize == cursor {
                    let vk = (packed & 0xFFFF) as u16;
                    let is_down = (packed >> 16) & 1 == 1;
                    let swallowed = (packed >> 17) & 1 == 1;
                    let flags = ((packed >> 24) & 0xFF) as u32;
                    let tag = if is_win(vk) { "WIN " } else { "key " };
                    say!(
                        "[{:>5}ms] {tag}{:<7} vk=0x{vk:02X} {} flags=0x{flags:02X}{}{}",
                        now_ms(),
                        if swallowed { "SWALLOW" } else { "PASS" },
                        if is_down { "down" } else { "up  " },
                        if flags & 0x10 != 0 { " INJECTED" } else { "" },
                        if flags & 0x01 != 0 { " EXTENDED" } else { "" }
                    );
                }
                cursor += 1;
            }
            if !RUNNING.load(Ordering::SeqCst) && cursor >= head {
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }
    });
}

fn is_win(vk: u16) -> bool {
    vk == 0x5B || vk == 0x5C
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: OpenProcess 只查询；句柄用完即关
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // PWSTR 在 windows-sys 里就是 *mut u16，直接给缓冲区指针
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

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: 只写入栈上缓冲
    let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: 只写入栈上缓冲
    let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// 盯 shell 界面：出现就打印，然后先 WM_CLOSE、不消失再 SW_HIDE，量化"按回去"的耗时
/// 盯前台窗口： 键唤出的开始菜单/搜索会抢走前台，这是唯一可靠的观测点
/// （实测：EnumWindows 看不到这些 XAML 岛窗口，不能靠枚举窗口类名判断）
fn spawn_shell_watcher() {
    thread::spawn(|| {
        let mut last: isize = 0;
        let mut hits = 0u32;

        loop {
            if !RUNNING.load(Ordering::SeqCst) {
                break;
            }

            // SAFETY: 只读前台窗口与其 pid
            unsafe {
                let fg = GetForegroundWindow();
                if !fg.is_null() && fg as isize != last {
                    last = fg as isize;
                    let mut pid: u32 = 0;
                    GetWindowThreadProcessId(fg, &mut pid);
                    let name = process_name(pid).unwrap_or_else(|| "?".into());
                    let shell = TARGETS.contains(&name.as_str());
                    if shell {
                        hits += 1;
                    }
                    say!(
                        "[{:>5}ms] [FG  ] {name:<28} class={} title={}{}",
                        now_ms(),
                        class_name(fg),
                        window_text(fg),
                        if shell {
                            "   <<< SHELL 抢到前台"
                        } else {
                            ""
                        }
                    );
                    let _ = std::io::stdout().flush();
                }
            }

            thread::sleep(Duration::from_millis(20));
        }

        say!("[watch] shell stole foreground {hits} time(s) in total");
    });
}

/// 注入一个 Esc：开始菜单/搜索打开时按 Esc 会自己收起，这是按回去最轻的手段
fn synth_esc() {
    let make = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0x1B,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let down = [make(false)];
    let up = [make(true)];
    // SAFETY: 两个单元素批次
    unsafe {
        SendInput(1, down.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
    thread::sleep(Duration::from_millis(30));
    // SAFETY: 同上
    unsafe {
        SendInput(1, up.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
    say!("[esc ] Esc injected");
}

/// 点一下任务栏开始按钮：这是唯一能**自动**唤出开始菜单的办法
/// （`SendInput` 合成的 `Win` 键唤不出，实测如此——历史文档的"合成能拦"是假对照）。
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
    say!("[click] clicked the bottom-left of the taskbar");
    let _ = std::io::stdout().flush();
    report_foreground();
}

/// 合成一个 `Win` 单击。`by_scancode` = 用扩展扫描码注入（部分系统只认这条路）
fn synth_win_tap(by_scancode: bool) {
    let flags = if by_scancode {
        KEYEVENTF_EXTENDEDKEY | KEYEVENTF_SCANCODE
    } else {
        0
    };
    let make = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: if by_scancode { 0 } else { VK_LWIN },
                wScan: if by_scancode { 0x5B } else { 0 },
                dwFlags: if up { flags | KEYEVENTF_KEYUP } else { flags },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let events = [make(false)];
    // SAFETY: 一个键盘事件
    let sent = unsafe {
        SendInput(
            events.len() as u32,
            events.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    // 按下与抬起**必须分开、留出间隔**：同一批里连着 down+up 会被当作瞬时抖动
    thread::sleep(Duration::from_millis(80));
    let events_up = [make(true)];
    // SAFETY: 一个键盘事件
    unsafe {
        SendInput(
            events_up.len() as u32,
            events_up.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    say!("[synth] SendInput returned {sent}/1 (down); key-up sent separately");
    report_foreground();
}

/// 打印当前前台窗口是谁：判断注入/点击有没有真的作用到用户的桌面上
fn report_foreground() {
    // SAFETY: 只读前台窗口与其 pid
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            say!("[fg] no foreground window");
            return;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(fg, &mut pid);
        let name = process_name(pid).unwrap_or_else(|| "?".into());
        say!(
            "[fg] foreground= {name} class={} title={}",
            class_name(fg),
            window_text(fg)
        );
        let _ = std::io::stdout().flush();
    }
}

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let synth = std::env::args().any(|a| a == "synth");
    let pass_win = std::env::args().any(|a| a == "pass");
    let click = std::env::args().any(|a| a == "click");
    let esc_after = std::env::args().any(|a| a == "esc");
    PASS_WIN.store(pass_win, Ordering::SeqCst);

    let log_path = std::env::temp_dir().join("biguansim-win-probe.log");
    init_log!(log_path.clone());

    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    START.store(epoch, Ordering::SeqCst);

    say!("== Win key diagnostic probe ==");
    say!("log also saved: {}", log_path.display());
    say!(
        "duration {seconds}s; quit with Ctrl+Alt+Shift+Q; synth={synth} pass={pass_win} (pass = let Win through)"
    );
    say!("step 1: optional synthetic Win tap (control group). step 2: press the REAL Win key 3 times.");
    say!("watch for: 1) WIN events  2) SHELL stole foreground  3) how long it took to push it back\n");
    let _ = std::io::stdout().flush();

    spawn_reporter();
    spawn_shell_watcher();

    if synth {
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(2));
            for round in 1..=2 {
                say!("\n---------- synthetic Win tap #{round} (VK injection) ----------");
                let _ = std::io::stdout().flush();
                synth_win_tap(false);
                thread::sleep(Duration::from_secs(2));
                say!("\n---------- synthetic Win tap #{round} (scancode injection) ----------");
                let _ = std::io::stdout().flush();
                synth_win_tap(true);
                thread::sleep(Duration::from_secs(2));
            }
        });
    }

    if click {
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            for round in 1..=2 {
                say!("\n---------- click the taskbar start button #{round} ----------");
                let _ = std::io::stdout().flush();
                click_start_button();
                if esc_after {
                    thread::sleep(Duration::from_millis(1200));
                    say!("---------- shell should be up, injecting Esc ----------");
                    synth_esc();
                }
                thread::sleep(Duration::from_secs(3));
            }
        });
    }

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(5));
        say!("\n---------- NOW press the REAL Win key 3 times (2s apart) ----------");
        let _ = std::io::stdout().flush();

        thread::sleep(Duration::from_secs(seconds.saturating_sub(5)));
        say!("\n[timeout] probe exiting.");
        let _ = std::io::stdout().flush();
        RUNNING.store(false, Ordering::SeqCst);
        let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if tid != 0 {
            // SAFETY: 向本进程的钩子线程投递退出消息
            unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
        }
    });

    HOOK_THREAD_ID.store(
        // SAFETY: 无参数、无副作用
        unsafe { GetCurrentThreadId() },
        Ordering::SeqCst,
    );

    // SAFETY: 取本模块句柄；钩子回调在进程生命周期内有效
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    // SAFETY: 在当前线程安装低级键盘钩子
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) };
    if hook.is_null() {
        eprintln!(
            "failed to install hook: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
    say!("hook installed.\n");
    let _ = std::io::stdout().flush();

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: 标准消息循环；msg 为栈上结构体
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        if !RUNNING.load(Ordering::SeqCst) {
            break;
        }
    }

    // SAFETY: 卸载自己安装的钩子
    unsafe { UnhookWindowsHookEx(hook) };
    say!("hook removed, probe exiting.");
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        // SAFETY: 交给下一个钩子
        return unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) };
    }

    let is_down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    let is_up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
    if !is_down && !is_up {
        // SAFETY: 交给下一个钩子
        return unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) };
    }

    // SAFETY: code >= 0 时系统保证 lparam 指向 KBDLLHOOKSTRUCT
    let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
    let vk = kb.vkCode as u16;
    let flags = kb.flags;

    let mut state = decode(MODS.load(Ordering::Relaxed));
    let verdict = decide(vk, is_down, &mut state);
    MODS.store(encode(&state), Ordering::Relaxed);

    // 安全出口
    if is_down && vk == 0x51 && state.ctrl && state.alt && state.shift && !state.any_win() {
        RUNNING.store(false, Ordering::SeqCst);
        // SAFETY: 向本进程钩子线程投递退出消息
        unsafe { PostThreadMessageW(HOOK_THREAD_ID.load(Ordering::SeqCst), WM_QUIT, 0, 0) };
        return 1;
    }

    push_event(vk, is_down, matches!(verdict, Verdict::Swallow), flags);

    if PASS_WIN.load(Ordering::Relaxed) && is_win(vk) {
        // SAFETY: 交给下一个钩子
        return unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) };
    }

    match verdict {
        Verdict::Swallow => 1,
        // SAFETY: 交给下一个钩子
        Verdict::Pass => unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) },
    }
}
