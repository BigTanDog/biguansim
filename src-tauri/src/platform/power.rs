//! 电源事件订阅（只订阅，不阻止睡眠——阻止睡眠属 M3）。
//!
//! 为什么必须订阅而不是"用墙钟差猜"：睡眠与"用户改系统时间"在数值上同形，
//! 只有独立的电源事件能把两者区分开（TDD §4.1 / §4.2）。

use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Power::{
    PowerRegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND,
    PBT_APMSUSPEND,
};

use super::time::{SystemMonotonic, SystemWall};
use crate::core::timer::{MonotonicClock, WallClock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerEventKind {
    Suspend,
    Resume,
}

#[derive(Clone, Copy, Debug)]
pub struct PowerEvent {
    pub kind: PowerEventKind,
    /// 事件发生时的单调时钟读数（毫秒）
    pub mono_ms: u64,
    /// 事件发生时的墙钟（Unix 毫秒）
    pub wall_ms: i64,
}

/// 回调里只做一次短促的入队。电源事件一个睡眠周期最多两条，
/// 不是每次按键都触发，因此这里用 mpsc 的短锁是可接受的；
/// 键盘钩子回调仍然严格禁止加锁（AGENTS §6）。
static SENDER: OnceLock<Sender<PowerEvent>> = OnceLock::new();

/// 注册挂起/恢复通知，返回事件接收端。
pub fn watch() -> Receiver<PowerEvent> {
    let (tx, rx) = mpsc::channel();
    if SENDER.set(tx).is_err() {
        // 已注册过：直接返回一个空接收端，避免重复注册
        let (_tx, rx) = mpsc::channel();
        return rx;
    }

    let params = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(callback),
        Context: std::ptr::null_mut(),
    });

    // 故意泄漏：注册回调的生命周期等同进程，Drop 掉会变成悬垂指针
    let params_ptr = Box::into_raw(params);
    let mut handle: *mut c_void = std::ptr::null_mut();

    // SAFETY: params_ptr 指向堆上长期有效的结构体；handle 只由系统写入
    let err = unsafe {
        PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK,
            params_ptr as HANDLE,
            &mut handle,
        )
    };

    if err != 0 {
        log::error!("电源通知注册失败，错误码 {err}");
    } else {
        log::info!("电源通知已注册");
    }

    rx
}

unsafe extern "system" fn callback(
    _context: *const c_void,
    event_type: u32,
    _setting: *const c_void,
) -> u32 {
    let kind = match event_type {
        PBT_APMSUSPEND => PowerEventKind::Suspend,
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL => {
            PowerEventKind::Resume
        }
        _ => return 0,
    };

    if let Some(sender) = SENDER.get() {
        let event = PowerEvent {
            kind,
            mono_ms: SystemMonotonic.unbiased_ms(),
            wall_ms: SystemWall.wall_ms(),
        };
        let _ = sender.send(event);
    }

    0
}
