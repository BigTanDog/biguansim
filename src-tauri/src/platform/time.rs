//! 系统时钟实现：单调时钟（不含睡眠）与墙钟（Unix 毫秒）。
//!
//! 两者都实现 `core::timer` 的 trait，Core 因此不需要认识 Win32（TDD §1.2）。

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;

use crate::core::timer::{MonotonicClock, WallClock};

/// 1601-01-01 到 1970-01-01 的 100ns 数，用于把 FILETIME 转 Unix 毫秒
const FILETIME_UNIX_EPOCH_DIFF: i64 = 116_444_736_000_000_000;

pub struct SystemMonotonic;

impl MonotonicClock for SystemMonotonic {
    fn unbiased_ms(&self) -> u64 {
        let mut ticks: u64 = 0;
        // SAFETY: 只写入栈上变量
        unsafe { QueryUnbiasedInterruptTime(&mut ticks) };
        ticks / 10_000
    }
}

pub struct SystemWall;

impl WallClock for SystemWall {
    fn wall_ms(&self) -> i64 {
        let mut ft = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        // SAFETY: 只写入栈上结构体
        unsafe { GetSystemTimePreciseAsFileTime(&mut ft) };

        let ticks = ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64;
        (ticks - FILETIME_UNIX_EPOCH_DIFF) / 10_000
    }
}
