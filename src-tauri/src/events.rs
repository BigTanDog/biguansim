//! IPC 推送载荷（TDD §5.2）。前端只渲染，不派生状态。

use serde::Serialize;

use crate::core::store::SessionResult;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChanged {
    pub state: String,
    pub result: Option<SessionResult>,
    pub goal: Option<String>,
    pub elapsed_ms: i64,
    pub target_ms: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerTick {
    pub elapsed_ms: i64,
    pub remaining_ms: i64,
    pub clock_skew_min: i64,
}

/// 起势倒计时（PRD F-2.1）。时间由 Rust 的 tick 线程给出，前端只渲染。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmingTick {
    pub remaining_ms: i64,
}

/// 钟鸣（PRD F-7.2）。`kind = "tick"` 是末 10 秒的轻滴答，`"bell"` 是到点的钟鸣。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chime {
    pub kind: &'static str,
    pub index: u32,
}

#[derive(Clone, Serialize)]
pub struct Toast {
    pub level: &'static str,
    pub text: String,
}
