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

#[derive(Clone, Serialize)]
pub struct Toast {
    pub level: &'static str,
    pub text: String,
}
