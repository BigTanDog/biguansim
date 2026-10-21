//! 系统时钟实现：单调时钟（不含睡眠）与墙钟（Unix 毫秒）。
//!
//! 两者都实现 `core::timer` 的 trait，Core 因此不需要认识 Win32（TDD §1.2）。

use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;

use crate::core::stats::LocalDate;
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

/// Unix 毫秒 → **本地**日期（统计按本地日归属，`decisions/003`）。
///
/// 用 `SystemTimeToTzSpecificLocalTime` 而不是自己减固定时差：夏令时、
/// 用户改时区都跟着系统走。转不出来（系统 API 失败）时退回 UTC 日期，
/// 只会让"跨午夜的那条记录"归错一天，不会崩溃。
pub fn local_date(wall_ms: i64) -> LocalDate {
    let ticks = wall_ms
        .saturating_mul(10_000)
        .saturating_add(FILETIME_UNIX_EPOCH_DIFF);
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };

    // SAFETY: 三个结构体都在栈上，指针有效
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&file_time, &mut utc) == 0 {
            return crate::core::stats::date_from_unix_ms_utc(wall_ms);
        }

        let mut local: SYSTEMTIME = std::mem::zeroed();
        if SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return crate::core::stats::date_from_unix_ms_utc(wall_ms);
        }

        LocalDate::new(local.wYear as i32, local.wMonth as u32, local.wDay as u32)
    }
}
