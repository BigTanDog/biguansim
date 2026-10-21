//! 低级键盘钩子（M3）。
//!
//! 四条硬约束：
//! 1. 回调里只做"取 vk + 判定 + 返回"，不分配、不加锁、不写文件、不打印（AGENTS §6）；
//! 2. 判定逻辑全部在 `keyfilter::decide`（纯函数），这里只做搬运；
//! 3. 60 s 周期重装，因为钩子被系统静默摘除时线程本身是活的，探针式检测发现不了；
//! 4. 每秒用 `GetAsyncKeyState` 校正修饰键状态——锁屏/安全桌面期间抬起事件不会回传，
//!    不校正的话解锁后键盘会被当成"Win 一直按住"而全部吞掉。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

use super::keyfilter::{
    decide, esc_consume as esc_consume_state, esc_hold_ms as esc_hold_at, esc_on_key, reconcile,
    wants_lock, EscBits, ModState, Verdict, VK_ESCAPE, VK_LWIN, VK_RWIN,
};

/// 自定义消息：让钩子线程在"最近 100 ms 没有按键"时重装钩子
const WM_REINSTALL: u32 = WM_APP + 1;
const REINSTALL_INTERVAL_SECS: u64 = 60;
const IDLE_BEFORE_REINSTALL_MS: u32 = 100;
/// 键态校正周期
const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);

const BIT_LWIN: u8 = 1 << 0;
const BIT_RWIN: u8 = 1 << 1;
const BIT_ALT: u8 = 1 << 2;
const BIT_CTRL: u8 = 1 << 3;
const BIT_SHIFT: u8 = 1 << 4;

static MODS: AtomicU8 = AtomicU8::new(0);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);
/// Esc 计时状态（一位打包，回调里 load/store，不加锁、不分配）
static ESC_BITS: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static SUPERVISOR_STARTED: AtomicBool = AtomicBool::new(false);
/// 待办动作：回调里**只置标志**，真正的阻塞调用（锁屏、发 toast）由专用线程做。
/// `LockWorkStation()` 要切安全桌面，耗时可达数百毫秒；在回调里做会拖过
/// `LowLevelHooksTimeout`，钩子被系统静默摘除——之后所有按键放行。
static PENDING_LOCK: AtomicBool = AtomicBool::new(false);
static PENDING_TOAST: AtomicBool = AtomicBool::new(false);
static ACTIONS_STARTED: AtomicBool = AtomicBool::new(false);
static APP: OnceLock<AppHandle> = OnceLock::new();

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
    }
}

fn encode(state: &ModState) -> u8 {
    let mut bits = 0u8;
    bits |= (state.lwin as u8) * BIT_LWIN;
    bits |= (state.rwin as u8) * BIT_RWIN;
    bits |= (state.alt as u8) * BIT_ALT;
    bits |= (state.ctrl as u8) * BIT_CTRL;
    bits |= (state.shift as u8) * BIT_SHIFT;
    bits
}

/// 清空按键状态：进入 Locked 时调用，避免起势期间按住的键串到闭关里。
pub fn reset_state() {
    MODS.store(0, Ordering::SeqCst);
    ESC_BITS.store(0, Ordering::SeqCst);
}

/// Esc 已按住的毫秒数；没按住（或被消费过）返回 0。M4 用它判定 8 秒破戒。
///
/// 注意：判定逻辑在 `keyfilter` 的纯函数里——长按时系统会疯狂重发 keydown，
/// 必须靠 armed 位只认"按下沿"，否则起点被不断刷新，永远到不了 8 秒。
pub fn esc_hold_ms() -> u64 {
    esc_hold_at(
        EscBits::from_raw(ESC_BITS.load(Ordering::SeqCst)),
        mono_ms(),
    )
}

/// 触发或取消破戒后清零：按住不放不会二次触发，必须松开再按（F-6.4）。
pub fn esc_consume() {
    let bits = EscBits::from_raw(ESC_BITS.load(Ordering::SeqCst));
    ESC_BITS.store(esc_consume_state(bits).raw(), Ordering::SeqCst);
}

/// 装钩子。失败必须让调用方拒绝入关（PRD D-7）。
pub fn install(app: &AppHandle) -> Result<(), String> {
    let _ = APP.set(app.clone());

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
                if let Err(err) = io_loop(hook) {
                    ::log::error!("[hook] 消息循环异常：{err}");
                    RUNNING.store(false, Ordering::SeqCst);
                    toast("键盘拦截异常，请重新入关");
                }
            }
            Err(err) => {
                let _ = tx.send(Err(err));
            }
        }
    });

    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => {
            spawn_supervisor();
            spawn_actions();
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
            return Err("消息循环异常".to_string());
        }
        if ret == 0 || !RUNNING.load(Ordering::SeqCst) {
            break;
        }

        if msg.message == WM_REINSTALL {
            // **先装新的、成功后再卸旧的**：两个低级钩子可以共存，
            // 先卸后装的话，一旦新钩子装不上，结界会在整个 60 s 周期里裸奔。
            match set_hook() {
                Ok(new_hook) => {
                    // SAFETY: 卸载旧的
                    unsafe { UnhookWindowsHookEx(hook as *mut _) };
                    hook = new_hook;
                    ::log::info!("[hook] 已重装");
                }
                Err(err) => {
                    ::log::warn!("[hook] 重装失败，继续使用旧钩子：{err}");
                    // toast 会走 IPC，不能在这个消息循环线程里做（会堵住钩子回调的投递）
                    PENDING_TOAST.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    // SAFETY: 卸载自己安装的钩子
    unsafe { UnhookWindowsHookEx(hook as *mut _) };
    Ok(())
}

/// 专用动作线程（**全进程只有一个**）：把"回调里不能做"的阻塞动作搬出来。
///
/// 为什么不用 channel：`Sender::send` 内部有锁与分配，回调里同样不许。
/// 50 ms 轮询，空闲时几乎不耗 CPU；`PENDING_*` 都是原子标志，回调侧只 store。
fn spawn_actions() {
    if ACTIONS_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    thread::spawn(|| loop {
        thread::sleep(Duration::from_millis(50));

        if PENDING_LOCK.swap(false, Ordering::SeqCst) {
            // SAFETY: 锁定当前工作站；失败也只是没锁上，结界仍在
            unsafe { LockWorkStation() };
            ::log::info!("[hook] 已代劳 Win+L 锁屏");
        }

        if PENDING_TOAST.swap(false, Ordering::SeqCst) {
            toast("键盘拦截重装失败，已保留旧钩子");
        }
    });
}

/// 常驻监督线程（**全进程只有一个**）：每秒用真实键态校正状态机，每 60 s 触发重装。
/// 之前是每次 install 都起一个，用户连开几关就会累积 N 个线程同时投递重装消息。
fn spawn_supervisor() {
    if SUPERVISOR_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    thread::spawn(|| {
        let mut seconds_since_reinstall: u64 = 0;

        loop {
            thread::sleep(RECONCILE_INTERVAL);

            if !RUNNING.load(Ordering::SeqCst) {
                seconds_since_reinstall = 0;
                continue;
            }

            // 1. 键态校正：锁屏/安全桌面期间的抬起事件收不到，只能靠真实键态纠正
            let pressed = |vk: i32| -> bool {
                // SAFETY: 只查询键态，无副作用
                (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
            };
            let mut state = decode(MODS.load(Ordering::Relaxed));
            reconcile(
                &mut state,
                pressed(VK_LWIN as i32),
                pressed(VK_RWIN as i32),
                pressed(super::keyfilter::VK_MENU as i32),
                pressed(super::keyfilter::VK_CONTROL as i32),
                pressed(super::keyfilter::VK_SHIFT as i32),
            );
            MODS.store(encode(&state), Ordering::Relaxed);

            // 2. 60 s 重装：挑最近没有按键的时刻
            seconds_since_reinstall += 1;
            if seconds_since_reinstall < REINSTALL_INTERVAL_SECS {
                continue;
            }

            let mut info = LASTINPUTINFO {
                cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            // SAFETY: 只写入栈上结构体
            unsafe { GetLastInputInfo(&mut info) };

            // SAFETY: 只读系统开机毫秒
            let idle_ms = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
                .wrapping_sub(info.dwTime);

            if idle_ms < IDLE_BEFORE_REINSTALL_MS {
                continue;
            }

            seconds_since_reinstall = 0;
            let tid = HOOK_TID.load(Ordering::SeqCst);
            if tid != 0 {
                // SAFETY: 向本进程的钩子线程投递自定义消息
                unsafe { PostThreadMessageW(tid, WM_REINSTALL, 0, 0) };
            }
        }
    });
}

fn toast(text: &str) {
    if let Some(app) = APP.get() {
        let _ = app.emit(
            "notice://toast",
            crate::events::Toast {
                level: "warn",
                text: text.to_string(),
            },
        );
    }
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
        let bits = EscBits::from_raw(ESC_BITS.load(Ordering::Relaxed));
        ESC_BITS.store(
            esc_on_key(bits, is_down, mono_ms()).raw(),
            Ordering::Relaxed,
        );
    }

    let mut state = decode(MODS.load(Ordering::Relaxed));
    let win_down = state.any_win();
    let verdict = decide(vk, is_down, &mut state);
    MODS.store(encode(&state), Ordering::Relaxed);

    // Win+L 被整体吞掉了，锁屏得我们自己来（F-3.4）。
    // **不能在这里直接调**：切安全桌面可能耗时数百毫秒，回调超时会让钩子被系统摘除。
    if wants_lock(vk, is_down, win_down) {
        PENDING_LOCK.store(true, Ordering::Relaxed);
    }

    match verdict {
        Verdict::Swallow => 1,
        // SAFETY: 交给下一个钩子
        Verdict::Pass => unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) },
    }
}
