//! 退出成本公式（PRD §7 / F-4.1 / F-5.2 / F-5.8）。
//!
//! 纯函数，不读配置、不碰时钟：`attempt` 由状态机给出，基数由 `config.json` 带入。
//! 三条口径必须守住：
//! 1. `attempts` 单调递增、答错不清零（PRD D-3）——成本只升不降；
//! 2. 冷却封顶 20 分钟、题数封顶 5 题（界面不公布公式，但数值必须与 §7 表一致）；
//! 3. 题数**下限 3**：2 题全对的蒙对率 6.25%，会让"蒙不过去"的论证失效（F-5.2）。

/// 每次尝试递增的冷却分钟数
pub const COOLDOWN_STEP_MIN: u32 = 5;
/// 冷却上限（分钟）
pub const COOLDOWN_CAP_MIN: u32 = 20;
/// 冷却基数的可设范围（F-10.4）
pub const COOLDOWN_BASE_MIN: u32 = 3;
pub const COOLDOWN_BASE_MAX: u32 = 10;
/// 题数上限与下限（F-5.2 / F-10.5）
pub const QUESTION_CAP: u32 = 5;
pub const QUESTION_FLOOR: u32 = 3;
/// 抄写字数（F-5.8）
pub const COPY_CHARS_THIRD: u32 = 100;
pub const COPY_CHARS_LATER: u32 = 200;

/// 第 `attempt` 次尝试的冷却（分钟）。`attempt` 从 1 开始；0 按 1 处理。
pub fn cooldown_min(attempt: u32, base_min: u32) -> u32 {
    let n = attempt.max(1);
    let base = base_min.clamp(COOLDOWN_BASE_MIN, COOLDOWN_BASE_MAX);
    base.saturating_add(COOLDOWN_STEP_MIN.saturating_mul(n - 1))
        .min(COOLDOWN_CAP_MIN)
}

pub fn cooldown_ms(attempt: u32, base_min: u32) -> u64 {
    u64::from(cooldown_min(attempt, base_min)) * 60_000
}

/// 第 `attempt` 次尝试要答的题数。`base` 取 `config.questionBase`（3–5）。
pub fn question_count(attempt: u32, base: u32) -> u32 {
    let n = attempt.max(1);
    let base = base.clamp(QUESTION_FLOOR, QUESTION_CAP);
    base.saturating_add(n - 1).min(QUESTION_CAP)
}

/// 第 `attempt` 次尝试全对后要抄的字数；0 表示不用抄。
pub fn copy_chars(attempt: u32) -> u32 {
    match attempt.max(1) {
        n if n >= 4 => COPY_CHARS_LATER,
        3 => COPY_CHARS_THIRD,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PRD §7 的表：5 / 10 / 15 / 20（封顶）
    #[test]
    fn cooldown_follows_prd_table() {
        let seq: Vec<u32> = (1..=6).map(|n| cooldown_min(n, 5)).collect();
        assert_eq!(seq, vec![5, 10, 15, 20, 20, 20]);
    }

    #[test]
    fn cooldown_uses_configured_base() {
        let seq: Vec<u32> = (1..=6).map(|n| cooldown_min(n, 3)).collect();
        assert_eq!(seq, vec![3, 8, 13, 18, 20, 20]);
    }

    /// 手改过配置（0 或 99）不得让成本变成 0 分钟
    #[test]
    fn cooldown_base_is_clamped() {
        assert_eq!(cooldown_min(1, 0), COOLDOWN_BASE_MIN);
        assert_eq!(cooldown_min(1, 99), COOLDOWN_BASE_MAX);
        assert_eq!(cooldown_min(4, 99), COOLDOWN_CAP_MIN);
    }

    #[test]
    fn attempt_zero_is_treated_as_first() {
        assert_eq!(cooldown_min(0, 5), 5);
        assert_eq!(question_count(0, 3), 3);
        assert_eq!(copy_chars(0), 0);
    }

    #[test]
    fn questions_follow_prd_table() {
        let seq: Vec<u32> = (1..=6).map(|n| question_count(n, 3)).collect();
        assert_eq!(seq, vec![3, 4, 5, 5, 5, 5]);
    }

    /// 下限 3：配置写成 1 也不许少于 3 题（蒙对率）
    #[test]
    fn questions_never_below_floor() {
        assert_eq!(question_count(1, 1), QUESTION_FLOOR);
        assert_eq!(question_count(1, 5), 5);
        assert_eq!(question_count(2, 5), QUESTION_CAP);
    }

    #[test]
    fn copy_chars_only_from_third_attempt() {
        assert_eq!(copy_chars(1), 0);
        assert_eq!(copy_chars(2), 0);
        assert_eq!(copy_chars(3), COPY_CHARS_THIRD);
        assert_eq!(copy_chars(4), COPY_CHARS_LATER);
        assert_eq!(copy_chars(9), COPY_CHARS_LATER);
    }

    #[test]
    fn cooldown_ms_matches_minutes() {
        assert_eq!(cooldown_ms(1, 5), 300_000);
        assert_eq!(cooldown_ms(2, 5), 600_000);
        assert_eq!(cooldown_ms(5, 5), 1_200_000);
    }
}
