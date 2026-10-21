//! IPC 推送载荷（TDD §5.2）。前端只渲染，不派生状态。

use serde::Serialize;

use crate::core::store::SessionResult;

/// 活跃态各自的投影。`state` 决定是哪一个，前端按 `kind` 分支渲染。
///
/// **不含答案**：`Challenge` 只有题干与打乱后的选项（A-25a）。
#[derive(Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PhasePayload {
    /// 冷却：`remainMs` 由 `cool://tick` 每 250 ms 刷新
    Cool {
        attempt: u32,
        cooldown_ms: i64,
        remain_ms: i64,
    },
    /// 问阵：`index` 从 1 起算，`id` 用于拒绝陈旧点击
    Challenge {
        id: String,
        index: usize,
        total: usize,
        stem: String,
        options: Vec<String>,
        attempt: u32,
    },
    /// 抄写：原文与字数（前端据此做展示层标红，通过与否只认 Rust）
    Copy { text: String, chars: u32 },
    /// 急事确认：要抄的 9 字与代价公示所需数字
    Confirm {
        sentence: String,
        elapsed_ms: i64,
        faults: u32,
    },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChanged {
    pub state: String,
    pub result: Option<SessionResult>,
    pub goal: Option<String>,
    pub elapsed_ms: i64,
    pub target_ms: i64,
    pub payload: Option<PhasePayload>,
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

/// 冷却倒计时（PRD F-4.3：界面只显示分钟粒度，这里给毫秒，格式化在前端）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoolTick {
    pub remain_ms: i64,
}

/// Esc 已按住的毫秒数（PRD F-6.1：进度环只由 Rust 给事实，前端不参与判定）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EscHold {
    pub held_ms: i64,
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
