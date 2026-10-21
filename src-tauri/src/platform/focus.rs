//! 抢焦点（M3 S3）：闭关期间前台一旦不是本进程，立刻抢回来。
//!
//! 两级机制：
//! - `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` 回调里**立即**抢回（F-3.5 要求 50 ms 内）；
//!   回调内只做 `SetForegroundWindow` 与一个原子计数，**不写日志、不加锁**（AGENTS §6）。
//! - 线程消息循环里每 500 ms 一次兜底复查：系统拒绝抢回时（前台进程有输入焦点保护），
//!   靠这个定时器持续重试而不是"等下一次事件"，否则用户可以一直待在别人窗口里。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, IsIconic, PostThreadMessageW,
    SetForegroundWindow, SetTimer, ShowWindow, EVENT_SYSTEM_FOREGROUND, MSG, SW_RESTORE,
    WINEVENT_OUTOFCONTEXT, WM_QUIT, WM_TIMER,
};

/// 兜底复查周期（TDD §4.5 的"降频到 500 ms 重试"）
const RETRY_INTERVAL_MS: u32 = 500;
/// 连续失败多少次后在日志里提示一次（之后不刷屏）
const WARN_AFTER_FAILURES: i32 = 3;

static RUNNING: AtomicBool = AtomicBool::new(false);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);
static TARGET: AtomicIsize = AtomicIsize::new(0);
static OUR_PID: AtomicU32 = AtomicU32::new(0);
/// 连续抢回失败次数（成功即清零）。回调里只 `fetch_add`，日志留到定时器里打。
static FAILURES: AtomicI32 = AtomicI32::new(0);
/// 已经为第几次连续失败打过日志，避免刷屏
static WARNED_AT: AtomicI32 = AtomicI32::new(0);
/// 从最小化状态还原的次数（回调里只累加，日志留到定时器里打）
static RESTORES: AtomicI32 = AtomicI32::new(0);

fn our_pid() -> u32 {
    if OUR_PID.load(Ordering::Relaxed) == 0 {
        OUR_PID.store(std::process::id(), Ordering::Relaxed);
    }
    OUR_PID.load(Ordering::Relaxed)
}

/// 启动抢焦点。**立即返回**，不阻塞调用方（tick 线程不能等）。
pub fn start(target: isize) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    TARGET.store(target, Ordering::SeqCst);
    FAILURES.store(0, Ordering::SeqCst);
    WARNED_AT.store(0, Ordering::SeqCst);

    thread::spawn(move || {
        HOOK_TID.store(
            // SAFETY: 无参数、无副作用
            unsafe { GetCurrentThreadId() },
            Ordering::SeqCst,
        );

        // SAFETY: 钩子在本线程安装，回调在进程生命周期内有效
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };

        if hook.is_null() {
            ::log::error!("[focus] 抢焦点钩子安装失败");
            RUNNING.store(false, Ordering::SeqCst);
            return;
        }

        // 线程定时器：没有窗口句柄时，WM_TIMER 直接投递到本线程的消息队列
        // SAFETY: 定时器与消息循环同线程
        unsafe { SetTimer(std::ptr::null_mut(), 0, RETRY_INTERVAL_MS, None) };

        ::log::info!("[focus] 抢焦点已启用，目标句柄 {target}，兜底复查 {RETRY_INTERVAL_MS} ms");

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            // SAFETY: 标准消息循环；msg 是栈上结构体
            let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if ret <= 0 || !RUNNING.load(Ordering::SeqCst) {
                break;
            }

            if msg.message == WM_TIMER {
                retry_once();
            }
        }

        // SAFETY: 卸载自己装的钩子
        unsafe { UnhookWinEvent(hook) };
        ::log::info!("[focus] 抢焦点线程已退出");
    });

    Ok(())
}

pub fn stop() {
    if !RUNNING.swap(false, Ordering::SeqCst) {
        return;
    }

    FAILURES.store(0, Ordering::SeqCst);
    WARNED_AT.store(0, Ordering::SeqCst);

    let tid = HOOK_TID.load(Ordering::SeqCst);
    if tid != 0 {
        // SAFETY: 向本进程的抢焦点线程投递退出消息
        unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
    }
    ::log::info!("[focus] 抢焦点已停止");
}

/// 把前台抢回我们的窗口。返回是否成功；调用方负责计数与日志。
fn try_reclaim() -> bool {
    let target = TARGET.load(Ordering::Relaxed) as HWND;
    if target.is_null() {
        return true;
    }

    // 「显示桌面」（Win+D）会把窗口最小化，而**最小化的窗口仍可能被记为前台**，
    // 于是下面"前台 pid 是本进程 → 已抢回"的判定会把这种情况当成正常，
    // 结果是桌面可见、键盘仍被钩子吞掉——2026-10-20 实测的"必须杀进程"就是这个。
    // 顺序不能改：必须先还原，再比 pid。
    // SAFETY: 只查询自己窗口的状态
    if unsafe { IsIconic(target) } != 0 {
        // SAFETY: 还原并置前自己的窗口；失败也无害，500 ms 的兜底会再试
        unsafe {
            ShowWindow(target, SW_RESTORE);
            SetForegroundWindow(target);
        }
        RESTORES.fetch_add(1, Ordering::Relaxed);
        return true;
    }

    // SAFETY: 只读当前前台窗口
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return true;
    }

    let mut pid: u32 = 0;
    // SAFETY: 只写入栈上变量
    unsafe { GetWindowThreadProcessId(foreground, &mut pid) };

    if pid == our_pid() {
        return true;
    }

    // SAFETY: 抢回自己的窗口
    let ok = unsafe { SetForegroundWindow(target) };
    if ok != 0 {
        return true;
    }

    // 前台进程持有输入焦点时，SetForegroundWindow 会直接失败（Windows 的前台锁）。
    // 用 AttachThreadInput 临时把自己接进前台线程的输入队列，再抢一次——这是标准做法。
    let mut foreground_pid_ignored: u32 = 0;
    // SAFETY: 只读前台线程 id
    let foreground_thread =
        unsafe { GetWindowThreadProcessId(foreground, &mut foreground_pid_ignored) };
    // SAFETY: 无参数
    let our_thread = unsafe { GetCurrentThreadId() };

    if foreground_thread == 0 || foreground_thread == our_thread {
        return false;
    }

    // SAFETY: 成对 attach/detach，中间只调 SetForegroundWindow
    unsafe {
        AttachThreadInput(our_thread, foreground_thread, 1);
        let forced = SetForegroundWindow(target);
        AttachThreadInput(our_thread, foreground_thread, 0);
        forced != 0
    }
}

fn note_result(ok: bool) -> i32 {
    if ok {
        FAILURES.store(0, Ordering::Relaxed);
        WARNED_AT.store(0, Ordering::Relaxed);
        return 0;
    }

    FAILURES.fetch_add(1, Ordering::Relaxed) + 1
}

/// 定时器里做的事：复查 + 打日志（这里不是钩子回调，允许 IO）
fn retry_once() {
    let restores = RESTORES.swap(0, Ordering::Relaxed);
    if restores > 0 {
        ::log::warn!("[focus] 窗口曾被最小化（显示桌面），已还原 {restores} 次");
    }

    let failures = note_result(try_reclaim());
    if failures >= WARN_AFTER_FAILURES && WARNED_AT.swap(failures, Ordering::Relaxed) != failures {
        ::log::warn!("[focus] 连续 {failures} 次抢回失败，继续每 {RETRY_INTERVAL_MS} ms 重试");
    }
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || !RUNNING.load(Ordering::Relaxed) {
        return;
    }

    // 回调内只做判定 + 抢回 + 原子计数，不写日志、不加锁
    let ok = try_reclaim();
    if !ok {
        FAILURES.fetch_add(1, Ordering::Relaxed);
    } else {
        FAILURES.store(0, Ordering::Relaxed);
    }
}

/// 供测试与排障：当前是否在运行
pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// 避免未使用告警：保留常量语义供外部读取
pub const fn retry_interval() -> Duration {
    Duration::from_millis(RETRY_INTERVAL_MS as u64)
}
