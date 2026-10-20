//! 低级键盘钩子（M3）。
//!
//! 三条硬约束（AGENTS §6 / TDD §4.4）：
//! 1. 回调里只做"取 vk + 判定 + 返回"，不分配、不加锁、不写文件、不打印；
//! 2. 判定逻辑全部在 `keyfilter::decide`（纯函数，19 条单测），这里只做搬运；
//! 3. 60 s 周期重装，因为钩子被系统静默摘除时线程本身是活的，探针式检测发现不了。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

use super::keyfilter::{decide, ModState, Verdict, VK_ESCAPE};

/// 自定义消息：让钩子线程在"最近 100 ms 没有按键"时重装钩子
const WM_REINSTALL: u32 = WM_APP + 1;
/// 重装周期
const REINSTALL_INTERVAL: Duration = Duration::from_secs(60);
/// 重装前要求的最小静默时间
const IDLE_BEFORE_REINSTALL_MS: u32 = 100;

const BIT_LWIN: u8 = 1 << 0;
const BIT_RWIN: u8 = 1 << 1;
const BIT_ALT: u8 = 1 << 2;
const BIT_CTRL: u8 = 1 << 3;
const BIT_SHIFT: u8 = 1 << 4;
const BIT_WIN_USED: u8 = 1 << 5;

static MODS: AtomicU8 = AtomicU8::new(0);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);
static ESC_DOWN_MS: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);

fn mono_ms() -> u64 {
    let mut ticks: u64 = 0;
    // SAFETY: 只写入栈上变量
    unsafe { QueryUnbiasedInterruptTime(&mut ticks) };
    ticks / 10_000
}

fn decode(bits: u8) -> ModState {
    ModState {
        lwin: bits & BIT_LWIN != 0,
        rwin: bits & BIT_RWIN != 0,
        alt: bits & BIT_ALT != 0,
        ctrl: bits & BIT_CTRL != 0,
        shift: bits & BIT_SHIFT != 0,
        win_used_other: bits & BIT_WIN_USED != 0,
    }
}

fn encode(state: &ModState) -> u8 {
    let mut bits = 0u8;
    bits |= (state.lwin as u8) * BIT_LWIN;
    bits |= (state.rwin as u8) * BIT_RWIN;
    bits |= (state.alt as u8) * BIT_ALT;
    bits |= (state.ctrl as u8) * BIT_CTRL;
    bits |= (state.shift as u8) * BIT_SHIFT;
    bits |= (state.win_used_other as u8) * BIT_WIN_USED;
    bits
}

/// 清空按键状态：进入 Locked 时调用，避免起势期间按住的键串到闭关里。
pub fn reset_state() {
    MODS.store(0, Ordering::SeqCst);
    ESC_DOWN_MS.store(0, Ordering::SeqCst);
}

/// Esc 已按住的毫秒数；没按住返回 0。M4 用它判定 8 秒破戒。
pub fn esc_hold_ms() -> u64 {
    let down = ESC_DOWN_MS.load(Ordering::SeqCst);
    if down == 0 {
        return 0;
    }
    mono_ms().saturating_sub(down)
}

/// 装钩子。失败必须让调用方拒绝入关（PRD D-7）。
pub fn install() -> Result<(), String> {
    if RUNNING.load(Ordering::SeqCst) {
        return Ok(());
    }

    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();

    thread::spawn(move || {
        HOOK_TID.store(
            // SAFETY: 无参数、无副作用
            unsafe { GetCurrentThreadId() },
            Ordering::SeqCst,
        );

        match set_hook() {
            Ok(hook) => {
                RUNNING.store(true, Ordering::SeqCst);
                let _ = tx.send(Ok(()));
                let _ = io_loop(hook);
            }
            Err(err) => {
                let _ = tx.send(Err(err));
            }
        }
    });

    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => {
            spawn_reinstaller();
            ::log::info!("[hook] 键盘钩子已安装");
            Ok(())
        }
        Ok(Err(err)) => Err(err),
        Err(_) => Err("钩子安装超时".to_string()),
    }
}

/// 卸载：只是让消息循环退出，钩子随线程结束被系统回收。
pub fn uninstall() {
    if !RUNNING.swap(false, Ordering::SeqCst) {
        return;
    }

    let tid = HOOK_TID.load(Ordering::SeqCst);
    if tid != 0 {
        // SAFETY: 向本进程的钩子线程投递退出消息
        unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
    }
    ::log::info!("[hook] 键盘钩子已卸载");
}

fn set_hook() -> Result<isize, String> {
    // SAFETY: GetModuleHandleW(None) 取本模块句柄
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    // SAFETY: 钩子在本线程安装，回调在进程生命周期内有效
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) };

    if hook.is_null() {
        let err = std::io::Error::last_os_error();
        ::log::error!("[hook] 安装失败：{err}");
        return Err(format!("E_HOOK_FAILED: {err}"));
    }
    Ok(hook as isize)
}

fn io_loop(mut hook: isize) -> Result<(), String> {
    let mut msg: MSG = unsafe { std::mem::zeroed() };

    loop {
        // SAFETY: 标准消息循环；msg 是栈上结构体
        let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };

        if ret == -1 {
            ::log::error!("[hook] 消息循环异常退出");
            return Err("E_HOOK_FAILED: 消息循环异常".to_string());
        }
        if ret == 0 || !RUNNING.load(Ordering::SeqCst) {
            break;
        }

        if msg.message == WM_REINSTALL {
            // 重装：先卸载再装，失败重试 3 次
            for attempt in 1..=3 {
                // SAFETY: 卸载自己装的钩子
                unsafe { UnhookWindowsHookEx(hook as *mut _) };
                match set_hook() {
                    Ok(new_hook) => {
                        hook = new_hook;
                        ::log::info!("[hook] 已重装（第 {attempt} 次尝试）");
                        break;
                    }
                    Err(err) => {
                        ::log::warn!("[hook] 重装失败（{attempt}/3）：{err}");
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        }
    }

    // SAFETY: 卸载自己安装的钩子
    unsafe { UnhookWindowsHookEx(hook as *mut _) };
    Ok(())
}

/// 每 60 s 提醒钩子线程重装一次；挑"最近没有按键"的时刻，避免吞掉真实按键。
fn spawn_reinstaller() {
    thread::spawn(|| loop {
        thread::sleep(REINSTALL_INTERVAL);
        if !RUNNING.load(Ordering::SeqCst) {
            return;
        }

        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: 只写入栈上结构体
        unsafe { GetLastInputInfo(&mut info) };

        // GetTickCount 与 dwTime 同为开机毫秒
        let idle_ms = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
            .wrapping_sub(info.dwTime);

        if idle_ms < IDLE_BEFORE_REINSTALL_MS {
            continue;
        }

        let tid = HOOK_TID.load(Ordering::SeqCst);
        if tid != 0 {
            // SAFETY: 向本进程的钩子线程投递自定义消息
            unsafe { PostThreadMessageW(tid, WM_REINSTALL, 0, 0) };
        }
    });
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

    if vk == VK_ESCAPE {
        if is_down {
            ESC_DOWN_MS.store(mono_ms(), Ordering::SeqCst);
        } else {
            ESC_DOWN_MS.store(0, Ordering::SeqCst);
        }
    }

    let mut state = decode(MODS.load(Ordering::Relaxed));
    let verdict = decide(vk, is_down, &mut state);
    MODS.store(encode(&state), Ordering::Relaxed);

    match verdict {
        Verdict::Swallow => 1,
        // SAFETY: 交给下一个钩子
        Verdict::Pass => unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) },
    }
}
