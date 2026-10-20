//! 状态机最小骨架（M1 只有 Idle / Locked / Settled，完整转移表在 M2）。
//!
//! 两条不可动摇的规则：
//! 1. 所有转移必须经过 `transition()` —— 唯一的入口（TDD §6.1）
//! 2. "到点是最高优先级"只在这里实现一次：只要有活跃会话且时间到，任何事件都先结算
//!    为 `completed`（PRD §5.2 全局规则）。判定必须先要求 `session.is_some()`，
//!    否则 Idle 下 remaining 天然为 0，一转移就被强制结算。

use crate::core::store::SessionResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Locked,
    Settled,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Idle => "Idle",
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
    Start {
        duration_ms: u64,
        now_ms: i64,
        goal: Option<String>,
        bank: String,
    },
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
    /// 进入 Locked
    Locked,
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
        // 到点优先：有活跃会话时，先判时间，再处理本次事件
        if let Some(session) = self.session.as_mut() {
            session.elapsed_ms = elapsed_ms;
            if elapsed_ms >= session.target_ms {
                return Ok(self.settle(SessionResult::Completed));
            }
        }

        match event {
            SessionEvent::Start {
                duration_ms,
                now_ms,
                goal,
                bank,
            } => {
                if self.state == SessionState::Locked {
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
                });
                self.state = SessionState::Locked;
                Ok(Outcome::Locked)
            }
            // 单纯的心跳不改状态；到点已在函数开头统一处理
            SessionEvent::Tick => Ok(Outcome::Unchanged),
            SessionEvent::Finish { result } => {
                if self.session.is_none() {
                    return Err(SessionError::StateIllegal);
                }
                Ok(self.settle(result))
            }
        }
    }

    /// 入关失败回滚：只在 `start_session` 的副作用失败时使用，
    /// 让状态回到"没有任何会话"的 Idle，避免半开半关。
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
mod tests {
    use super::*;

    fn machine() -> SessionMachine {
        SessionMachine::new()
    }

    fn start(machine: &mut SessionMachine, duration_ms: u64) {
        let outcome = machine
            .transition(
                SessionEvent::Start {
                    duration_ms,
                    now_ms: 1_760_875_920_000,
                    goal: Some("背完 List 5–7".to_string()),
                    bank: "basic".to_string(),
                },
                0,
            )
            .expect("入关应当成功");
        assert!(matches!(outcome, Outcome::Locked));
    }

    #[test]
    fn start_from_idle_enters_locked() {
        let mut m = machine();
        start(&mut m, 60_000);

        assert_eq!(m.state(), SessionState::Locked);
        assert_eq!(m.session().unwrap().id, "s_1760875920000");
    }

    #[test]
    fn double_start_is_illegal() {
        let mut m = machine();
        start(&mut m, 60_000);

        let err = m.transition(
            SessionEvent::Start {
                duration_ms: 60_000,
                now_ms: 1,
                goal: None,
                bank: "basic".to_string(),
            },
            0,
        );

        assert_eq!(err.unwrap_err(), SessionError::StateIllegal);
    }

    /// 回归：Idle 下 remaining 天然为 0，到点判定若不带 session 前置会误结算
    #[test]
    fn tick_in_idle_does_not_settle() {
        let mut m = machine();

        let outcome = m
            .transition(SessionEvent::Tick, 0)
            .expect("Idle 下的 tick 不是非法事件");

        assert!(matches!(outcome, Outcome::Unchanged));
        assert_eq!(m.state(), SessionState::Idle);
    }

    #[test]
    fn tick_before_deadline_keeps_locked() {
        let mut m = machine();
        start(&mut m, 60_000);

        let outcome = m
            .transition(SessionEvent::Tick, 59_999)
            .expect("tick");

        assert!(matches!(outcome, Outcome::Unchanged));
        assert_eq!(m.state(), SessionState::Locked);
        assert_eq!(m.session().unwrap().remaining_ms(), 1);
    }

    #[test]
    fn deadline_settles_as_completed() {
        let mut m = machine();
        start(&mut m, 60_000);

        let outcome = m
            .transition(SessionEvent::Tick, 60_000)
            .expect("tick");

        match outcome {
            Outcome::Settled { result, session } => {
                assert_eq!(result, SessionResult::Completed);
                assert_eq!(session.elapsed_ms, 60_000);
            }
            _ => panic!("到点必须结算"),
        }
        assert_eq!(m.state(), SessionState::Settled);
        assert!(m.session().is_none());
    }

    /// 到点优先于"主动结束"：Finish 在到点后也必须是 completed
    #[test]
    fn deadline_wins_over_explicit_finish() {
        let mut m = machine();
        start(&mut m, 60_000);

        let outcome = m
            .transition(
                SessionEvent::Finish {
                    result: SessionResult::Emergency,
                },
                90_000,
            )
            .expect("finish");

        match outcome {
            Outcome::Settled { result, .. } => assert_eq!(result, SessionResult::Completed),
            _ => panic!("必须结算"),
        }
    }

    #[test]
    fn finish_without_session_is_illegal() {
        let mut m = machine();

        let err = m
            .transition(
                SessionEvent::Finish {
                    result: SessionResult::Completed,
                },
                1_000,
            )
            .unwrap_err();

        assert_eq!(err, SessionError::StateIllegal);
    }

    #[test]
    fn can_start_again_after_settle() {
        let mut m = machine();
        start(&mut m, 1_000);
        let _ = m
            .transition(SessionEvent::Tick, 1_000)
            .expect("tick");
        assert_eq!(m.state(), SessionState::Settled);

        start(&mut m, 2_000);
        assert_eq!(m.state(), SessionState::Locked);
    }
}
