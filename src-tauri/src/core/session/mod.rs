//! 状态机（八态：Idle / Arming / Locked / Cool / Challenge / Copying / Confirming / Settled）。
//!
//! 四条不可动摇的规则：
//! 1. 所有转移必须经过 `transition()` —— 唯一的入口（TDD §6.1）。
//! 2. **到点是最高优先级**：活跃态（`is_active()`）下只要 `elapsed >= target`，
//!    任何事件都先结算为 `completed`。这条只在这里实现一次，不得在 Cool/Challenge
//!    分支里各写一遍。`Arming` 不参与（计时器还没启动）。
//! 3. `Arming` 是起势的 3 秒窗口：只有 `ArmFinished` / `Cancel` 两个出口。
//! 4. 成本（冷却分钟数、题数、抄写字数）只由 `core::cost` 计算，且**只升不降**：
//!    `attempts` 单调递增，答错不清零（PRD D-3）。
//!
//! 题目与抄写原文都由调用方（`runtime`）在转移时喂进来，本模块不读文件、不抽题：
//! 它只负责"能不能转、转完是什么"。

use crate::core::challenge::{Judgement, Round};
use crate::core::copy::{self, CopyTask};
use crate::core::cost;
use crate::core::store::SessionResult;

/// 起势时长（PRD F-2.1：3-2-1，每步 1 秒）
pub const ARMING_MS: u64 = 3_000;
/// 急事破戒要逐字抄的 9 个字（PRD F-6.2）
pub const EMERGENCY_SENTENCE: &str = "我确认放弃本次闭关";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Arming,
    Locked,
    Cool,
    Challenge,
    Copying,
    Confirming,
    Settled,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Idle => "Idle",
            SessionState::Arming => "Arming",
            SessionState::Locked => "Locked",
            SessionState::Cool => "Cool",
            SessionState::Challenge => "Challenge",
            SessionState::Copying => "Copying",
            SessionState::Confirming => "Confirming",
            SessionState::Settled => "Settled",
        }
    }

    /// 活跃态：计时器在跑、到点规则生效、结界必须还立着、离开时必须走 teardown。
    pub fn is_active(self) -> bool {
        matches!(
            self,
            SessionState::Locked
                | SessionState::Cool
                | SessionState::Challenge
                | SessionState::Copying
                | SessionState::Confirming
        )
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
    /// 冷却基数（分钟）与题数基数，入关时从 `config.json` 带入，避免状态机读文件
    pub cooldown_base: u32,
    pub question_base: u32,
    /// 冷却缩放：正式版 1，测试版 10（5 分钟 → 30 秒）。
    /// 只影响冷却时长，不影响题数/抄写字数——为了让自测能在一个短会话里走到第 3 轮。
    pub cost_scale: u32,
    /// 本次闭关的说话人（阵灵/前辈/冥冥之中的声音/长老，入关时随机）
    pub speaker: String,
    /// 冷却结束时的 `elapsed` 读数。用单调口径而不是墙钟：改系统时间不能缩短冷却。
    pub cool_end_elapsed_ms: Option<u64>,
    /// 本轮冷却的总时长（已缩放），投影直接用它，避免前端自己算
    pub cool_ms: u64,
    pub round: Option<Round>,
    pub copy: Option<CopyTask>,
}

impl SessionInfo {
    pub fn remaining_ms(&self) -> u64 {
        self.target_ms.saturating_sub(self.elapsed_ms)
    }

    /// 冷却剩余毫秒；不在冷却中返回 None
    pub fn cooldown_remain_ms(&self, elapsed_ms: u64) -> Option<u64> {
        self.cool_end_elapsed_ms
            .map(|end| end.saturating_sub(elapsed_ms))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    /// 非法转移（TDD §5.3 的 E_STATE_ILLEGAL）
    StateIllegal,
    /// 选项索引越界或题号对不上（E_INVALID_CHOICE）
    InvalidChoice,
    /// 破戒抄写句不一致（E_CONFIRM_MISMATCH）
    ConfirmMismatch,
    /// 该抄写了却没有原文：只在题库/语料构建缺陷时出现
    CopyMissing,
    /// 抽到的题为空：绝不进入"无题可问"的 Challenge（PRD §12）
    EmptyRound,
}

pub enum SessionEvent {
    /// Idle/Settled → Arming
    Start {
        duration_ms: u64,
        now_ms: i64,
        now_mono_ms: u64,
        goal: Option<String>,
        bank: String,
        cooldown_base: u32,
        question_base: u32,
        cost_scale: u32,
        speaker: String,
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
    /// Locked → Cool（可能被拒：剩余不足一次冷却）
    RequestUnlock,
    /// Cool → Challenge：冷却到点后由调用方抽好题一起送来
    CoolFinished {
        round: Round,
    },
    /// Challenge 内提交一次选择；`copy` 只在需要抄写时给出
    SubmitChoice {
        choice: usize,
        copy: Option<CopyTask>,
    },
    /// Copying 内提交抄写内容
    SubmitCopy {
        text: String,
    },
    /// Locked → Confirming（调用方已校验 Esc 满 8 秒）
    RequestEmergency,
    /// Confirming → Settled(emergency)（文本由本模块校验）
    ConfirmEmergency {
        text: String,
    },
    /// Confirming → Locked
    CancelEmergency,
}

#[derive(Debug)]
pub enum Outcome {
    /// 状态没变，只是刷新了投影
    Unchanged,
    /// 进入起势
    Arming,
    /// 起势结束，正式闭关（调用方要启动计时器）
    Locked,
    /// 起势被取消 / 破戒被取消，回到可交互态
    Idle,
    /// 进入冷却（`cooldown_ms` 是本次冷却总长）
    Cool { attempt: u32, cooldown_ms: u64 },
    /// 剩余不足一次冷却：不转移，界面提示"请稍候到点"（F-4.5 / A-22）
    UnlockRefused { need_ms: u64, remaining_ms: u64 },
    /// 冷却到点，调用方该抽题了
    CooldownElapsed,
    /// 进入问阵
    Challenge,
    /// 答对一题且后面还有题
    ChallengeAdvanced,
    /// 进入抄写（`target_chars` 是本次要求的字数：3 → 100，4 及以上 → 200）
    Copying { target_chars: u32 },
    /// 抄写不一致：不转移，返回首个错字位置
    CopyMismatch { index: usize },
    /// 进入破戒确认
    Confirming,
    /// 离开活跃态，需要执行 teardown。
    /// `session` 装箱：`SessionInfo` 带着题目与抄写任务，直接内联会让整个枚举变胖。
    Settled {
        result: SessionResult,
        session: Box<SessionInfo>,
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

    /// 本次尝试若全对，需要抄多少字（0 = 直接出关）。调用方用它决定要不要取原文。
    pub fn copy_chars(&self) -> u32 {
        self.session
            .as_ref()
            .map(|session| cost::copy_chars(session.attempts))
            .unwrap_or(0)
    }

    /// 唯一入口。`elapsed_ms` 由调用方给出当前读数（tick 线程持有计时器）。
    pub fn transition(
        &mut self,
        event: SessionEvent,
        elapsed_ms: u64,
    ) -> Result<Outcome, SessionError> {
        // 到点优先：活跃态下任何事件都先结算（PRD §5.2 全局规则）
        if self.state.is_active() {
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
                cooldown_base,
                question_base,
                cost_scale,
                speaker,
            } => {
                if self.state.is_active() || self.state == SessionState::Arming {
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
                    cooldown_base,
                    question_base,
                    cost_scale: cost_scale.max(1),
                    speaker,
                    cool_end_elapsed_ms: None,
                    cool_ms: 0,
                    round: None,
                    copy: None,
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
            SessionEvent::Tick => {
                // 冷却到点不自动转移：抽题要访问题库，交给调用方做（保持本模块纯逻辑）
                if self.state == SessionState::Cool {
                    if let Some(session) = self.session.as_ref() {
                        if let Some(end) = session.cool_end_elapsed_ms {
                            if elapsed_ms >= end {
                                return Ok(Outcome::CooldownElapsed);
                            }
                        }
                    }
                }
                Ok(Outcome::Unchanged)
            }
            SessionEvent::Finish { result } => {
                if !self.state.is_active() || self.session.is_none() {
                    return Err(SessionError::StateIllegal);
                }
                // 显示变化允许从任一活跃态出关（M4 计划 D-26）；
                // 其余结果各有专属路径，不许从这里伪造
                match result {
                    SessionResult::Accident => Ok(self.settle(SessionResult::Accident)),
                    _ => Err(SessionError::StateIllegal),
                }
            }
            SessionEvent::RequestUnlock => {
                if self.state != SessionState::Locked {
                    return Err(SessionError::StateIllegal);
                }
                let Some(session) = self.session.as_mut() else {
                    return Err(SessionError::StateIllegal);
                };

                let next_attempt = session.attempts.saturating_add(1);
                let need = cooldown_of(session, next_attempt);
                let remaining = session.remaining_ms();
                if remaining < need {
                    return Ok(Outcome::UnlockRefused {
                        need_ms: need,
                        remaining_ms: remaining,
                    });
                }

                session.attempts = next_attempt;
                session.cool_end_elapsed_ms = Some(elapsed_ms.saturating_add(need));
                session.cool_ms = need;
                self.state = SessionState::Cool;
                Ok(Outcome::Cool {
                    attempt: next_attempt,
                    cooldown_ms: need,
                })
            }
            SessionEvent::CoolFinished { round } => {
                if self.state != SessionState::Cool {
                    return Err(SessionError::StateIllegal);
                }
                let Some(session) = self.session.as_mut() else {
                    return Err(SessionError::StateIllegal);
                };

                // 冷却没到点不许进问阵：否则前端一个 invoke 就能跳过 5 分钟
                match session.cool_end_elapsed_ms {
                    Some(end) if elapsed_ms >= end => {}
                    _ => return Err(SessionError::StateIllegal),
                }
                if round.is_empty() {
                    return Err(SessionError::EmptyRound);
                }

                session.round = Some(round);
                self.state = SessionState::Challenge;
                Ok(Outcome::Challenge)
            }
            SessionEvent::SubmitChoice { choice, copy } => {
                if self.state != SessionState::Challenge {
                    return Err(SessionError::StateIllegal);
                }
                let Some(session) = self.session.as_mut() else {
                    return Err(SessionError::StateIllegal);
                };

                // **先确认抄写原文在，再动题目**：`Round::submit` 会推进题号，
                // 先提交再报错就回滚不了，会留下"状态 Challenge、题号已越界"的死态
                // （2026-10-21 审阅指出）。语料缺失属于构建缺陷，直接拒绝本次作答。
                let needs_copy = cost::copy_chars(session.attempts) > 0;
                if needs_copy && copy.is_none() {
                    ::log::error!("[session] 需要抄写却没有原文，拒绝本次作答（语料构建缺陷）");
                    return Err(SessionError::CopyMissing);
                }

                let Some(round) = session.round.as_mut() else {
                    return Err(SessionError::StateIllegal);
                };

                match round.submit(choice) {
                    Judgement::Invalid => Err(SessionError::InvalidChoice),
                    Judgement::Next => Ok(Outcome::ChallengeAdvanced),
                    Judgement::Wrong => {
                        // F-5.5 / F-5.7：不公布答案，成本 +1，回冷却
                        session.round = None;
                        session.attempts = session.attempts.saturating_add(1);
                        let need = cooldown_of(session, session.attempts);
                        session.cool_end_elapsed_ms = Some(elapsed_ms.saturating_add(need));
                        session.cool_ms = need;
                        self.state = SessionState::Cool;
                        Ok(Outcome::Cool {
                            attempt: session.attempts,
                            cooldown_ms: need,
                        })
                    }
                    Judgement::AllCorrect => {
                        let target_chars = cost::copy_chars(session.attempts);
                        session.round = None;

                        if target_chars == 0 {
                            return Ok(self.settle(SessionResult::Early));
                        }

                        // 上面已经拦过"需要抄写却没原文"，这里不可能为空
                        let Some(task) = copy else {
                            return Err(SessionError::CopyMissing);
                        };

                        session.copy = Some(task);
                        self.state = SessionState::Copying;
                        Ok(Outcome::Copying { target_chars })
                    }
                }
            }
            SessionEvent::SubmitCopy { text } => {
                if self.state != SessionState::Copying {
                    return Err(SessionError::StateIllegal);
                }
                let Some(session) = self.session.as_mut() else {
                    return Err(SessionError::StateIllegal);
                };
                let Some(task) = session.copy.as_ref() else {
                    return Err(SessionError::StateIllegal);
                };

                match copy::check(&task.text, &text) {
                    copy::CopyVerdict::Ok => {
                        session.copy = None;
                        Ok(self.settle(SessionResult::Early))
                    }
                    copy::CopyVerdict::Mismatch { index } => Ok(Outcome::CopyMismatch { index }),
                }
            }
            SessionEvent::RequestEmergency => {
                if self.state != SessionState::Locked {
                    return Err(SessionError::StateIllegal);
                }
                self.state = SessionState::Confirming;
                Ok(Outcome::Confirming)
            }
            SessionEvent::ConfirmEmergency { text } => {
                if self.state != SessionState::Confirming {
                    return Err(SessionError::StateIllegal);
                }
                if text != EMERGENCY_SENTENCE {
                    return Err(SessionError::ConfirmMismatch);
                }
                if let Some(session) = self.session.as_mut() {
                    session.faults = session.faults.saturating_add(1);
                }
                Ok(self.settle(SessionResult::Emergency))
            }
            SessionEvent::CancelEmergency => {
                if self.state != SessionState::Confirming {
                    return Err(SessionError::StateIllegal);
                }
                self.state = SessionState::Locked;
                Ok(Outcome::Locked)
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
        session.round = None;
        session.copy = None;
        Outcome::Settled {
            result,
            session: Box::new(session),
        }
    }
}

/// 本次尝试的冷却时长（已按 `cost_scale` 缩放，且不低于 1 秒）。
/// 缩放只用于 debug 构建自测，正式版 `cost_scale = 1`。
fn cooldown_of(session: &SessionInfo, attempt: u32) -> u64 {
    let raw = cost::cooldown_ms(attempt, session.cooldown_base);
    (raw / u64::from(session.cost_scale.max(1))).max(1_000)
}

#[cfg(test)]
mod tests;
