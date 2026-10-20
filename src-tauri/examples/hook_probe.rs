//! 拦键探针（M1 S10 spike 用，不进主程序）。
//!
//! 目的：用同一份 `keyfilter::decide()` 验证五项系统行为——
//! 1) Win+L 能锁屏　2) Win+E 被拦　3) Alt+Tab 被拦
//! 4) 单按 Win 不弹开始菜单、且不残留"Win 被按住"状态　5) 正常打字与输入法切换不受影响
//!
//! 安全出口（三选一，任何情况下都能退出）：
//! - 按 `Ctrl+Alt+Shift+Q`
//! - 启动 5 分钟后自动退出
//! - 在另一个终端 `taskkill /IM hook_probe.exe /F`
//!
//! 运行：`cargo run --manifest-path src-tauri/Cargo.toml --example hook_probe`

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

use biguansim_lib::platform::keyfilter::{decide, ModState, Verdict};

/// 修饰键状态用位图存在原子里：钩子回调里不加锁（AGENTS §6）
const BIT_LWIN: u8 = 1 << 0;
const BIT_RWIN: u8 = 1 << 1;
const BIT_ALT: u8 = 1 << 2;
const BIT_CTRL: u8 = 1 << 3;
const BIT_SHIFT: u8 = 1 << 4;

static MODS: AtomicU8 = AtomicU8::new(0);
static RUNNING: AtomicBool = AtomicBool::new(true);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// 回调里**不允许**打印：控制台写入可达毫秒级，一旦超过 `LowLevelHooksTimeout`
/// （默认 1 s）钩子会被系统静默摘除，SP-3/SP-5 就会"假失败"。
/// 所以回调只往这个无锁环里塞一条记录，另开线程读出来打印。
const RING_LEN: usize = 256;
static RING: [AtomicU64; RING_LEN] = [const { AtomicU64::new(0) }; RING_LEN];
static RING_HEAD: AtomicUsize = AtomicUsize::new(0);

fn push_event(vk: u16, is_down: bool, swallowed: bool) {
    let seq = RING_HEAD.fetch_add(1, Ordering::Relaxed);
    // 高 32 位存序号：读侧靠它区分"这一格是不是本轮写的"，避免与覆盖竞争
    let packed = ((seq as u64) << 32)
        | ((swallowed as u64) << 17)
        | ((is_down as u64) << 16)
        | vk as u64;
    RING[seq % RING_LEN].store(packed, Ordering::Release);
}

/// 把环里的按键记录搬到 stdout，与钩子回调解耦
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
                    println!(
                        "{:<7} vk=0x{vk:02X} {}",
                        if swallowed { "SWALLOW" } else { "PASS" },
                        if is_down { "down" } else { "up  " }
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

fn main() {
    println!("拦键探针已启动。");
    println!("安全出口：Ctrl+Alt+Shift+Q，或 5 分钟后自动退出。");
    println!("每按一个键都会打印 PASS/SWALLOW，请按清单逐条试。");

    // 打印放在独立线程：钩子回调里只写无锁环，绝不碰 stdout
    spawn_reporter();

    // 5 分钟自毁：即便钩子把退出组合键也吞了，也不会把人锁住
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(300));
        println!("\n[自动退出] 已到 5 分钟上限。");
        RUNNING.store(false, Ordering::SeqCst);
        let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if tid != 0 {
            // SAFETY: 只是向本进程的钩子线程投递退出消息
            unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
        }
    });

    HOOK_THREAD_ID.store(
        // SAFETY: GetCurrentThreadId 无参数、无副作用
        unsafe { GetCurrentThreadId() },
        Ordering::SeqCst,
    );

    // SAFETY: GetModuleHandleW(None) 取本模块句柄；调用期间字符串指针为空
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    // SAFETY: 钩子在当前线程安装，回调函数在进程生命周期内有效
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) };

    if hook.is_null() {
        eprintln!(
            "安装钩子失败（错误码 {}），请检查安全软件是否拦截。",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }

    println!("钩子已安装，开始监听。\n");

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: 标准消息循环；msg 是栈上结构体
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        if !RUNNING.load(Ordering::SeqCst) {
            break;
        }
    }

    // SAFETY: 卸载自己安装的钩子
    unsafe { UnhookWindowsHookEx(hook) };
    println!("钩子已卸载，探针退出。");
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        // SAFETY: 交给下一个钩子处理
        return unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) };
    }

    let is_down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    let is_up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);

    if !is_down && !is_up {
        // SAFETY: 交给下一个钩子处理
        return unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) };
    }

    // SAFETY: 系统保证 code >= 0 时 lparam 指向 KBDLLHOOKSTRUCT
    let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
    let vk = kb.vkCode as u16;

    let mut state = decode(MODS.load(Ordering::Relaxed));
    let verdict = decide(vk, is_down, &mut state);
    MODS.store(encode(&state), Ordering::Relaxed);

    // 安全出口：Ctrl+Alt+Shift+Q。只做原子操作与投递消息，不打印
    if is_down && vk == 0x51 && state.ctrl && state.alt && state.shift && !state.any_win() {
        RUNNING.store(false, Ordering::SeqCst);
        PostThreadMessageW(HOOK_THREAD_ID.load(Ordering::SeqCst), WM_QUIT, 0, 0);
        return 1;
    }

    push_event(vk, is_down, matches!(verdict, Verdict::Swallow));

    match verdict {
        Verdict::Swallow => 1,
        // SAFETY: 交给下一个钩子处理
        Verdict::Pass => unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) },
    }
}
