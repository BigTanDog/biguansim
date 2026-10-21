//! 记录统计：累计 / 有效日 / 连续天数 / 破戒（PRD F-9.4 ~ F-9.7）。
//!
//! 全程纯函数：调用方把每条记录的 `startedAtMs` 换算成本地日期后传进来
//! （换算在 `platform::time`，用 Win32 时区 API），所以本模块不碰系统时间，
//! 测试可以随便造"昨天/前天/跨年"。
//!
//! 三条最容易写错的口径，都在这里钉死：
//! 1. **有效日按 `startedAtMs` 归属**，一条跨午夜的记录只算一天，不拆；
//! 2. `accident` / `interrupted` 计累计但不计有效日，`emergency` 两者都不计；
//! 3. 连续天数：今天没达标不算断签（给当天留机会）；今天和昨天都没有有效日才是 0。

use std::collections::BTreeSet;

use crate::core::store::SessionResult;

/// 一个"有效日"的门槛（F-9.5）
pub const VALID_DAY_MS: i64 = 25 * 60_000;

/// 本地日历日（不含时区信息；换算发生在 `platform::time`）
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl LocalDate {
    pub fn new(year: i32, month: u32, day: u32) -> LocalDate {
        LocalDate { year, month, day }
    }

    /// 前一天（跨月/跨年都对）
    pub fn yesterday(self) -> LocalDate {
        let days = days_from_civil(self.year, self.month, self.day);
        let (year, month, day) = civil_from_days(days - 1);
        LocalDate { year, month, day }
    }
}

/// 一条记录在统计层的形状
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatedRecord {
    pub date: LocalDate,
    pub result: SessionResult,
    pub actual_ms: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// 累计时长：completed / early / accident / interrupted
    pub total_ms: i64,
    pub sessions: u32,
    /// 破戒：emergency + accident
    pub faults: u32,
    /// 历史有效日总数
    pub valid_days: u32,
    /// 当前连续有效日
    pub streak_days: u32,
    /// 今天是否已达标
    pub today_valid: bool,
}

pub fn summary(records: &[DatedRecord], today: LocalDate) -> Summary {
    let mut total_ms = 0i64;
    let mut faults = 0u32;
    let mut valid_ms_by_day: std::collections::BTreeMap<LocalDate, i64> = Default::default();

    for record in records {
        if record.result != SessionResult::Emergency {
            total_ms = total_ms.saturating_add(record.actual_ms.max(0));
        }
        if matches!(
            record.result,
            SessionResult::Emergency | SessionResult::Accident
        ) {
            faults = faults.saturating_add(1);
        }
        if matches!(
            record.result,
            SessionResult::Completed | SessionResult::Early
        ) {
            let slot = valid_ms_by_day.entry(record.date).or_insert(0);
            *slot = slot.saturating_add(record.actual_ms.max(0));
        }
    }

    let valid_days: BTreeSet<LocalDate> = valid_ms_by_day
        .iter()
        .filter(|(_, ms)| **ms >= VALID_DAY_MS)
        .map(|(date, _)| *date)
        .collect();

    Summary {
        total_ms,
        sessions: records.len() as u32,
        faults,
        valid_days: valid_days.len() as u32,
        streak_days: streak(&valid_days, today),
        today_valid: valid_days.contains(&today),
    }
}

/// 连续有效日（F-9.6）：今天达标就从今天数，否则从昨天数；昨天也没有就是 0。
pub fn streak(valid: &BTreeSet<LocalDate>, today: LocalDate) -> u32 {
    let mut cursor = if valid.contains(&today) {
        Some(today)
    } else {
        let yesterday = today.yesterday();
        if valid.contains(&yesterday) {
            Some(yesterday)
        } else {
            None
        }
    };

    let mut count = 0;
    while let Some(date) = cursor {
        if !valid.contains(&date) {
            break;
        }
        count += 1;
        cursor = Some(date.yesterday());
    }
    count
}

// ---------- 公历 ↔ 天数（Howard Hinnant 的 civil 算法，纯整数、无依赖） ----------

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = (year - era * 400) as i64;
    let month = month as i64;
    let day = day as i64;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era as i64 * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 60 * 60 * 1_000;

    fn record(date: LocalDate, result: SessionResult, minutes: i64) -> DatedRecord {
        DatedRecord {
            date,
            result,
            actual_ms: minutes * 60_000,
        }
    }

    #[test]
    fn yesterday_handles_month_and_year_edges() {
        assert_eq!(
            LocalDate::new(2026, 3, 1).yesterday(),
            LocalDate::new(2026, 2, 28)
        );
        assert_eq!(
            LocalDate::new(2024, 3, 1).yesterday(),
            LocalDate::new(2024, 2, 29)
        );
        assert_eq!(
            LocalDate::new(2026, 1, 1).yesterday(),
            LocalDate::new(2025, 12, 31)
        );
    }

    /// F-9.5：24:59 不算有效日，25:00 算
    #[test]
    fn valid_day_threshold_is_25_minutes() {
        let today = LocalDate::new(2026, 10, 21);
        let short = summary(&[record(today, SessionResult::Completed, 24)], today);
        assert!(!short.today_valid);
        assert_eq!(short.valid_days, 0);

        let long = summary(&[record(today, SessionResult::Completed, 25)], today);
        assert!(long.today_valid);
        assert_eq!(long.valid_days, 1);
    }

    /// 当日多次累加：10 + 16 = 26 分钟 → 有效日
    #[test]
    fn same_day_sessions_accumulate() {
        let today = LocalDate::new(2026, 10, 21);
        let stats = summary(
            &[
                record(today, SessionResult::Completed, 10),
                record(today, SessionResult::Early, 16),
            ],
            today,
        );
        assert!(stats.today_valid);
        assert_eq!(stats.total_ms, 26 * 60_000);
    }

    /// F-9.4：accident 计累计不计有效日；emergency 两者都不计
    #[test]
    fn accident_counts_total_but_not_valid_day() {
        let today = LocalDate::new(2026, 10, 21);
        let stats = summary(
            &[
                record(today, SessionResult::Accident, 60),
                record(today, SessionResult::Emergency, 40),
            ],
            today,
        );
        assert_eq!(stats.total_ms, 60 * 60_000, "emergency 不计累计");
        assert!(!stats.today_valid, "accident 不计有效日");
        assert_eq!(stats.faults, 2, "emergency 与 accident 都算破戒");
    }

    /// F-9.6：今天没达标但昨天达标 → 连续天数仍算（当天不断签）
    #[test]
    fn streak_survives_a_not_yet_valid_today() {
        let today = LocalDate::new(2026, 10, 21);
        let yesterday = today.yesterday();
        let stats = summary(&[record(yesterday, SessionResult::Completed, 30)], today);

        assert_eq!(stats.streak_days, 1);
        assert!(!stats.today_valid);
    }

    /// F-9.6 / A-18：今天与昨天都没有有效日 → 0（已断签）
    #[test]
    fn streak_is_zero_after_a_gap() {
        let today = LocalDate::new(2026, 10, 21);
        let two_days_ago = today.yesterday().yesterday();
        let stats = summary(&[record(two_days_ago, SessionResult::Completed, 30)], today);

        assert_eq!(stats.streak_days, 0);
        assert_eq!(stats.valid_days, 1, "历史有效日仍然保留");
    }

    #[test]
    fn streak_counts_consecutive_days() {
        let today = LocalDate::new(2026, 10, 21);
        let records = vec![
            record(today, SessionResult::Completed, 30),
            record(today.yesterday(), SessionResult::Completed, 30),
            record(today.yesterday().yesterday(), SessionResult::Early, 30),
        ];
        assert_eq!(summary(&records, today).streak_days, 3);
    }

    /// 跨午夜：23:50 开始的一条只属于开始那天（不拆、不四舍五入）
    #[test]
    fn late_night_record_belongs_to_its_start_day() {
        let today = LocalDate::new(2026, 10, 21);
        let yesterday = today.yesterday();
        let stats = summary(&[record(yesterday, SessionResult::Completed, 30)], today);

        assert_eq!(stats.valid_days, 1);
        assert!(!stats.today_valid, "不能算到今天头上");
        assert_eq!(stats.streak_days, 1);
    }

    #[test]
    fn empty_history_is_all_zero() {
        let stats = summary(&[], LocalDate::new(2026, 10, 21));
        assert_eq!(stats, Summary::default());
    }

    /// 日期换算与天数往返一致（跨闰年、跨世纪都覆盖）
    #[test]
    fn civil_round_trip() {
        for (year, month, day) in [
            (1970, 1, 1),
            (2000, 2, 29),
            (2024, 12, 31),
            (2026, 10, 21),
            (2100, 3, 1),
        ] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
        }

        assert_eq!(
            days_from_civil(2026, 10, 21) - days_from_civil(2026, 10, 20),
            1
        );
        assert_eq!(DAY, 86_400_000);
    }
}
