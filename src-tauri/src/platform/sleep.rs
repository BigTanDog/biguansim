//! 阻止系统睡眠（M3 S4）。
//!
//! `SetThreadExecutionState` 的请求是**线程级**的：设置与清除必须在同一个线程，
//! 所以这里专门开一条线程持有请求，主逻辑只负责发"开始/停止"信号。
//!
//! 只加 `ES_SYSTEM_REQUIRED` 不加 `ES_DISPLAY_REQUIRED`：闭关中允许熄屏（PRD F-3.8 / D-5）。

use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
};

static STOP: Mutex<Option<Sender<()>>> = Mutex::new(None);

/// 开始阻止睡眠；重复调用是幂等的。
pub fn block() {
    let mut guard = match STOP.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };

    if guard.is_some() {
        return;
    }

    let (tx, rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<bool>();

    thread::spawn(move || {
        // SAFETY: 在本线程设置，也在本线程清除
        let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        let ok = previous != 0;
        let _ = ready_tx.send(ok);

        if !ok {
            ::log::error!("[sleep] SetThreadExecutionState 失败");
            return;
        }

        // 一直挂到收到停止信号
        let _ = rx.recv_timeout(Duration::from_secs(60 * 60 * 12));

        // SAFETY: 同一线程清除
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        ::log::info!("[sleep] 已解除睡眠阻止");
    });

    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(true) => {
            *guard = Some(tx);
            ::log::info!("[sleep] 已阻止系统睡眠");
        }
        _ => ::log::error!("[sleep] 睡眠阻止未能生效"),
    }
}

/// 解除阻止；重复调用是幂等的。
pub fn allow() {
    let sender = match STOP.lock() {
        Ok(mut guard) => guard.take(),
        Err(_) => None,
    };

    if let Some(tx) = sender {
        let _ = tx.send(());
    }
}
