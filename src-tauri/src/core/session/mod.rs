//! 状态机（M2：Idle / Arming / Locked / Settled，完整转移表在 M4）。
//!
//! 三条不可动摇的规则：
//! 1. 所有转移必须经过 `transition()` —— 唯一的入口（TDD §6.1）
//! 2. "到点是最高优先级"只在这里实现一次：**只要有活跃会话、处于 Locked 且时间到**，
//!    任何事件都先结算为 `completed`（PRD §5.2）。判定必须先要求 `session.is_some()`
//!    **且 `state == Locked`**：Idle 下 remaining 天然为 0，Arming 下计时器还没启动，
//!    少了这个前置就会误结算。
//! 3. Arming 是起势的 3 秒窗口：此时还没有计时器，只有 `ArmFinished` / `Cancel` 两个出口。
//!
//! 生产代码与测试分文件（`tests.rs`），M4 扩到八态时两边一起变长。

use crate::core::store::SessionResult;

/// 起势时长（PRD F-2.1：3-2-1，每步 1 秒）
pub const ARMING_MS: u64 = 3_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Arming,
    Locked,
    Settled,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Idle => "Idle",
            SessionState::Arming => "Arming",
            SessionState::Locked => "Locked",
            SessionState::Settled => "Settled",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: String,
    pub started_at_ms: i64,
    pub target_ms: u64,
    pub elapsed_ms: u64,
    pub goal: Option<String>,
    pub bank: String,
    pub attempts: u32,
    pub faults: u32,
    /// 起势开始时的单调时钟读数，仅 Arming 期间有意义
    pub armed_at_mono_ms: u64,
}

impl SessionInfo {
    pub fn remaining_ms(&self) -> u64 {
        self.target_ms.saturating_sub(self.elapsed_ms)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    /// 非法转移（TDD §5.3 的 E_STATE_ILLEGAL）
    StateIllegal,
}

pub enum SessionEvent {
    /// Idle/Settled → Arming
    Start {
        duration_ms: u64,
        now_ms: i64,
        now_mono_ms: u64,
        goal: Option<String>,
        bank: String,
    },
    /// Arming → Locked（3 秒走完）
    ArmFinished,
    /// Arming → Idle（起势窗口内取消）
    Cancel,
    Tick,
    /// 到点或异常主动结算
    Finish {
        result: SessionResult,
    },
}

#[derive(Debug)]
pub enum Outcome {
    /// 状态没变，只是刷新了投影
    Unchanged,
    /// 进入起势
    Arming,
    /// 起势结束，正式闭关（调用方要启动计时器）
    Locked,
    /// 起势被取消，回到配置页
    Idle,
    /// 离开 Locked，需要执行 teardown
    Settled {
        result: SessionResult,
        session: SessionInfo,
    },
}

pub struct SessionMachine {
    state: SessionState,
    session: Option<SessionInfo>,
}

impl Default for SessionMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionMachine {
    pub fn new() -> Self {
        Self {
            state: SessionState::Idle,
            session: None,
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn session(&self) -> Option<&SessionInfo> {
        self.session.as_ref()
    }

    /// 唯一入口。`elapsed_ms` 由调用方给出当前读数（tick 线程持有计时器）。
    pub fn transition(
        &mut self,
        event: SessionEvent,
        elapsed_ms: u64,
    ) -> Result<Outcome, SessionError> {
        // 到点优先：只有正在闭关（计时器已启动）时才可能"到点"
        if self.state == SessionState::Locked {
            if let Some(session) = self.session.as_mut() {
                session.elapsed_ms = elapsed_ms;
                if elapsed_ms >= session.target_ms {
                    return Ok(self.settle(SessionResult::Completed));
                }
            }
        }

        match event {
            SessionEvent::Start {
                duration_ms,
                now_ms,
                now_mono_ms,
                goal,
                bank,
            } => {
                if self.state == SessionState::Arming || self.state == SessionState::Locked {
                    return Err(SessionError::StateIllegal);
                }

                self.session = Some(SessionInfo {
                    id: format!("s_{now_ms}"),
                    started_at_ms: now_ms,
                    target_ms: duration_ms,
                    elapsed_ms: 0,
                    goal,
                    bank,
                    attempts: 0,
                    faults: 0,
                    armed_at_mono_ms: now_mono_ms,
                });
                self.state = SessionState::Arming;
                Ok(Outcome::Arming)
            }
            SessionEvent::ArmFinished => {
                if self.state != SessionState::Arming {
                    return Err(SessionError::StateIllegal);
                }
                self.state = SessionState::Locked;
                Ok(Outcome::Locked)
            }
            SessionEvent::Cancel => {
                if self.state != SessionState::Arming {
                    return Err(SessionError::StateIllegal);
                }
                self.reset();
                Ok(Outcome::Idle)
            }
            // 单纯的心跳不改状态；到点已在函数开头统一处理
            SessionEvent::Tick => Ok(Outcome::Unchanged),
            SessionEvent::Finish { result } => {
                if self.state != SessionState::Locked || self.session.is_none() {
                    return Err(SessionError::StateIllegal);
                }
                Ok(self.settle(result))
            }
        }
    }

    /// 记一次故障（显示变化等），用于 `records.jsonl` 的 `faults` 字段（PRD §5.2）。
    pub fn note_fault(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.faults = session.faults.saturating_add(1);
        }
    }

    /// 入关失败/取消时回滚：回到"没有任何会话"的 Idle，避免半开半关。
    pub fn reset(&mut self) {
        self.state = SessionState::Idle;
        self.session = None;
    }

    fn settle(&mut self, result: SessionResult) -> Outcome {
        let Some(mut session) = self.session.take() else {
            return Outcome::Unchanged;
        };

        self.state = SessionState::Settled;
        session.elapsed_ms = session.elapsed_ms.min(session.target_ms);
        Outcome::Settled { result, session }
    }
}

#[cfg(test)]
mod tests;
