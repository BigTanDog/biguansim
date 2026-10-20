//! 计时：单调时钟为主，睡眠事件单独累加（TDD §4.1 / PRD F-3.15、F-3.16）。
//!
//! 关键性质：
//! - 系统时间被调快/调慢 → `elapsed_ms` 不变（改时间不影响结算）
//! - 传统睡眠 → 恢复时按差分补上睡掉的时间
//! - Modern Standby（mono 与 wall 同步前进）→ 差分≈0，不会重复累加
//!
//! 时钟通过 trait 注入，测试用 FakeClock，禁止真实 sleep（AGENTS §5）。

/// 不受系统时间调整影响的单调时钟（毫秒）
pub trait MonotonicClock {
    fn unbiased_ms(&self) -> u64;
}

/// 墙钟（Unix 毫秒）。只用于计算睡眠时长与检测时间篡改，不参与结算。
pub trait WallClock {
    fn wall_ms(&self) -> i64;
}

/// 时间篡改提示阈值：超过 3 分钟且无睡眠事件时提示"不一致"
pub const SKEW_THRESHOLD_MIN: i64 = 3;

pub struct Timer<M: MonotonicClock, W: WallClock> {
    monotonic: M,
    wall: W,
    mono_base: u64,
    wall_base: i64,
    /// 睡眠期间单调时钟没走的那部分时长
    sleep_accum_ms: u64,
    /// 挂起时刻的 (mono, wall)
    suspended_at: Option<(u64, i64)>,
}

impl<M: MonotonicClock, W: WallClock> Timer<M, W> {
    /// 以"现在"为原点开始计时（进入 Locked 时调用）
    pub fn start(monotonic: M, wall: W) -> Self {
        let mono_base = monotonic.unbiased_ms();
        let wall_base = wall.wall_ms();
        Self {
            monotonic,
            wall,
            mono_base,
            wall_base,
            sleep_accum_ms: 0,
            suspended_at: None,
        }
    }

    /// 已坚持时长 = 单调增量 + 睡眠累计
    pub fn elapsed_ms(&self) -> u64 {
        let mono_delta = self.monotonic.unbiased_ms().saturating_sub(self.mono_base);
        mono_delta.saturating_add(self.sleep_accum_ms)
    }

    /// 剩余时长（不足则为 0）
    pub fn remaining_ms(&self, target_ms: u64) -> u64 {
        target_ms.saturating_sub(self.elapsed_ms())
    }

    /// 墙钟与结算时长的偏差（分钟）。只用于界面提示，不参与结算。
    pub fn skew_min(&self) -> i64 {
        let wall_delta = self.wall.wall_ms().saturating_sub(self.wall_base);
        let elapsed = self.elapsed_ms() as i64;
        (wall_delta - elapsed) / 60_000
    }

    /// 是否需要把"不一致"提示出来
    pub fn is_clock_suspect(&self) -> bool {
        self.skew_min().abs() > SKEW_THRESHOLD_MIN
    }

    /// 收到挂起通知。重复的挂起通知不覆盖第一次的时刻。
    pub fn on_suspend(&mut self) {
        if self.suspended_at.is_none() {
            self.suspended_at = Some((self.monotonic.unbiased_ms(), self.wall.wall_ms()));
        }
    }

    /// 收到恢复通知（`RESUMEAUTOMATIC` 与 `RESUMESUSPEND` 都走这里）。
    ///
    /// 用差分而不是直接用墙钟差：Modern Standby 下单调时钟也在前进，
    /// 若直接加墙钟差就会把 10 分钟记成 20 分钟。
    pub fn on_resume(&mut self) {
        let Some((suspend_mono, suspend_wall)) = self.suspended_at.take() else {
            // 没收到挂起就来恢复（例如启动时就已在运行），忽略
            return;
        };

        let mono_delta = self.monotonic.unbiased_ms().saturating_sub(suspend_mono) as i64;
        let wall_delta = self.wall.wall_ms().saturating_sub(suspend_wall);
        let missed = wall_delta - mono_delta;

        if missed > 0 {
            self.sleep_accum_ms = self.sleep_accum_ms.saturating_add(missed as u64);
        }
    }

    /// 快照用的单调基准（写入 session.json，重启后仅作诊断）
    pub fn mono_base(&self) -> u64 {
        self.mono_base
    }

    /// 快照用的墙钟基准（Unix 毫秒）
    pub fn wall_base_ms(&self) -> i64 {
        self.wall_base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct FakeClock {
        mono_ms: Cell<u64>,
        wall_ms: Cell<i64>,
    }

    impl FakeClock {
        fn new(mono_ms: u64, wall_ms: i64) -> Rc<Self> {
            Rc::new(Self {
                mono_ms: Cell::new(mono_ms),
                wall_ms: Cell::new(wall_ms),
            })
        }

        /// 正常时间流逝：单调与墙钟同步前进
        fn advance(&self, ms: u64) {
            self.mono_ms.set(self.mono_ms.get() + ms);
            self.wall_ms.set(self.wall_ms.get() + ms as i64);
        }

        /// 传统 S3 睡眠：只有墙钟前进，单调时钟几乎不动
        fn sleep_s3(&self, ms: u64) {
            self.wall_ms.set(self.wall_ms.get() + ms as i64);
        }

        /// Modern Standby：单调与墙钟一起前进（内核仍在走）
        fn sleep_s0ix(&self, ms: u64) {
            self.advance(ms);
        }

        /// 用户手动改系统时间
        fn shift_wall(&self, ms: i64) {
            self.wall_ms.set(self.wall_ms.get() + ms);
        }
    }

    impl MonotonicClock for Rc<FakeClock> {
        fn unbiased_ms(&self) -> u64 {
            self.mono_ms.get()
        }
    }

    impl WallClock for Rc<FakeClock> {
        fn wall_ms(&self) -> i64 {
            self.wall_ms.get()
        }
    }

    const BASE_WALL: i64 = 1_760_875_920_000;

    fn started() -> (Rc<FakeClock>, Timer<Rc<FakeClock>, Rc<FakeClock>>) {
        let clock = FakeClock::new(1_000_000, BASE_WALL);
        let timer = Timer::start(Rc::clone(&clock), Rc::clone(&clock));
        (clock, timer)
    }

    #[test]
    fn elapsed_grows_with_time() {
        let (clock, timer) = started();
        assert_eq!(timer.elapsed_ms(), 0);

        clock.advance(1_000);
        assert_eq!(timer.elapsed_ms(), 1_000);

        clock.advance(59_000);
        assert_eq!(timer.elapsed_ms(), 60_000);
    }

    #[test]
    fn wall_clock_forward_does_not_grant_time() {
        let (clock, timer) = started();
        clock.advance(10_000);
        clock.shift_wall(3_600_000);

        assert_eq!(timer.elapsed_ms(), 10_000);
        assert_eq!(timer.skew_min(), 60);
        assert!(timer.is_clock_suspect());
    }

    #[test]
    fn wall_clock_backward_does_not_lose_time() {
        let (clock, timer) = started();
        clock.advance(10_000);
        clock.shift_wall(-600_000);

        assert_eq!(timer.elapsed_ms(), 10_000);
        assert_eq!(timer.skew_min(), -10);
        assert!(timer.is_clock_suspect());
    }

    #[test]
    fn small_skew_is_not_suspect() {
        let (clock, timer) = started();
        clock.advance(10_000);
        clock.shift_wall(120_000); // +2 分钟，阈值内

        assert_eq!(timer.skew_min(), 2);
        assert!(!timer.is_clock_suspect());
    }

    #[test]
    fn s3_sleep_counts_full_wall_delta() {
        let (clock, mut timer) = started();
        clock.advance(5_000);

        timer.on_suspend();
        clock.sleep_s3(600_000); // 睡 10 分钟，单调时钟没动
        timer.on_resume();

        assert_eq!(timer.elapsed_ms(), 605_000);
    }

    #[test]
    fn modern_standby_is_not_double_counted() {
        let (clock, mut timer) = started();
        clock.advance(5_000);

        timer.on_suspend();
        clock.sleep_s0ix(600_000); // mono 与 wall 同步前进
        timer.on_resume();

        // 只算 10 分钟，不是 20 分钟
        assert_eq!(timer.elapsed_ms(), 605_000);
    }

    #[test]
    fn resume_without_suspend_is_ignored() {
        let (clock, mut timer) = started();
        clock.advance(30_000);

        timer.on_resume();

        assert_eq!(timer.elapsed_ms(), 30_000);
    }

    #[test]
    fn repeated_suspend_resume_accumulates_without_overflow() {
        let (clock, mut timer) = started();

        timer.on_suspend();
        timer.on_suspend(); // 重复挂起通知不覆盖
        clock.sleep_s3(60_000);
        timer.on_resume();
        assert_eq!(timer.elapsed_ms(), 60_000);

        clock.advance(1_000);
        timer.on_suspend();
        clock.sleep_s3(120_000);
        timer.on_resume();

        assert_eq!(timer.elapsed_ms(), 181_000);
    }

    #[test]
    fn remaining_never_underflows() {
        let (clock, timer) = started();
        clock.advance(120_000);

        assert_eq!(timer.remaining_ms(60_000), 0);
        assert_eq!(timer.remaining_ms(180_000), 60_000);
    }
}
