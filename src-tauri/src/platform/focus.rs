//! 抢焦点（M3 S3）：闭关期间前台一旦不是本进程，立刻抢回来。
//!
//! 用 `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)`：
//! 事件投递到**安装钩子的那个线程**的消息队列，因此回调里做一次
//! `SetForegroundWindow` 是安全的（不跨线程、不阻塞主线程）。
//!
//! 连续抢不回时降频重试而不是放弃——放弃等于结界破了。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, PostThreadMessageW,
    SetForegroundWindow, EVENT_SYSTEM_FOREGROUND, MSG, WINEVENT_OUTOFCONTEXT, WM_QUIT,
};

use windows_sys::Win32::System::Threading::GetCurrentThreadId;

const MAX_CONSECUTIVE_FAILURES: u32 = 3;

static RUNNING: AtomicBool = AtomicBool::new(false);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);
static OUR_PID: AtomicU32 = AtomicU32::new(0);
static RECLAIM_FAILURES: AtomicU32 = AtomicU32::new(0);

fn our_pid() -> u32 {
    if OUR_PID.load(Ordering::Relaxed) == 0 {
        OUR_PID.store(std::process::id(), Ordering::Relaxed);
    }
    OUR_PID.load(Ordering::Relaxed)
}

/// 启动抢焦点。`target` 是主窗口句柄（抢回时用它）。
pub fn start(target: isize) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let (tx, rx) = std::sync::mpsc::channel::<bool>();

    thread::spawn(move || {
        HOOK_TID.store(
            // SAFETY: 无参数、无副作用
            unsafe { GetCurrentThreadId() },
            Ordering::SeqCst,
        );

        TARGET.store(target, Ordering::SeqCst);

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
            let _ = tx.send(false);
            return;
        }

        let _ = tx.send(true);

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            // SAFETY: 标准消息循环；msg 是栈上结构体
            let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if ret <= 0 || !RUNNING.load(Ordering::SeqCst) {
                break;
            }
        }

        // SAFETY: 卸载自己装的钩子
        unsafe { UnhookWinEvent(hook) };
        ::log::info!("[focus] 抢焦点线程已退出");
    });

    match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(true) => {
            ::log::info!("[focus] 抢焦点已启用，目标句柄 {target}");
            Ok(())
        }
        Ok(false) | Err(_) => Err("E_FOCUS_FAILED: 抢焦点钩子未能安装".to_string()),
    }
}

pub fn stop() {
    if !RUNNING.swap(false, Ordering::SeqCst) {
        return;
    }
    RECLAIM_FAILURES.store(0, Ordering::SeqCst);

    let tid = HOOK_TID.load(Ordering::SeqCst);
    if tid != 0 {
        // SAFETY: 向本进程的钩子线程投递退出消息
        unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
    }
    ::log::info!("[focus] 抢焦点已停止");
}

static TARGET: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

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

    // SAFETY: 只读当前前台窗口
    let foreground = unsafe { GetForegroundWindow() };

    if foreground.is_null() {
        return;
    }

    let mut pid: u32 = 0;
    // SAFETY: 只写入栈上变量
    unsafe { GetWindowThreadProcessId(foreground, &mut pid) };

    if pid == our_pid() {
        RECLAIM_FAILURES.store(0, Ordering::Relaxed);
        return;
    }

    let target = TARGET.load(Ordering::Relaxed) as HWND;
    if target.is_null() {
        return;
    }

    // SAFETY: 目标句柄由 Tauri 提供，抢回自己的窗口
    let ok = unsafe { SetForegroundWindow(target) };

    if ok != 0 {
        RECLAIM_FAILURES.store(0, Ordering::Relaxed);
    } else {
        let failures = RECLAIM_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
        if failures == MAX_CONSECUTIVE_FAILURES {
            ::log::warn!(
                "[focus] 连续 {failures} 次抢回失败（前台 pid={pid}），保持重试但不刷日志"
            );
        }
    }
}
