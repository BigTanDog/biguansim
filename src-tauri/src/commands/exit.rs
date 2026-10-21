//! 退出流程的五个命令（PRD F-4.x / F-5.x / F-6.x）。
//!
//! 三条口径：
//! 1. **判定只在这里和状态机里**：前端发的是意图（点了解锁、选了第几个），
//!    对错、成本、能否进入下一阶段全部由 Rust 决定；
//! 2. 任何非预期状态一律 `E_STATE_ILLEGAL`，不静默成功（TDD §5.1）；
//! 3. 一旦转移导致结算，必须立刻走 `finish()`（出关善后只有这一处）。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::core::copy::CopyTask;
use crate::core::rng::Rng;
use crate::core::session::{Outcome, SessionError, SessionEvent};
use crate::runtime::{finish, publish_state, read_timer, Runtime};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockView {
    /// false 只在"点下去的那一刻恰好到点"出现：此时已结算，前端什么都不用做
    accepted: bool,
    attempt: u32,
    cooldown_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceView {
    accepted: bool,
    correct: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyView {
    ok: bool,
    /// 首个错字位置，供界面标红
    mismatch_index: Option<usize>,
}

fn map_error(err: SessionError) -> String {
    match err {
        SessionError::StateIllegal => "E_STATE_ILLEGAL".to_string(),
        SessionError::InvalidChoice => "E_INVALID_CHOICE".to_string(),
        SessionError::ConfirmMismatch => "E_CONFIRM_MISMATCH".to_string(),
        SessionError::CopyMissing => {
            ::log::error!("[exit] 需要抄写却没有原文（语料构建缺陷）");
            "E_STATE_ILLEGAL".to_string()
        }
        SessionError::EmptyRound => {
            ::log::error!("[exit] 抽到空题（题库缺陷）");
            "E_BANK_INVALID".to_string()
        }
    }
}

fn elapsed_of(runtime: &Arc<Runtime>) -> u64 {
    read_timer(runtime).map(|(elapsed, _)| elapsed).unwrap_or(0)
}

/// Locked → Cool（PRD F-4.1 / F-4.5）
#[tauri::command]
pub fn request_unlock(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
) -> Result<UnlockView, String> {
    let elapsed = elapsed_of(&runtime);
    let outcome = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;
        machine.transition(SessionEvent::RequestUnlock, elapsed)
    };

    match outcome {
        Ok(Outcome::Cool {
            attempt,
            cooldown_ms,
        }) => {
            publish_state(&app, &runtime);
            ::log::info!(
                "[exit] 第 {attempt} 次解锁：冷却 {} 分钟",
                cooldown_ms / 60_000
            );
            Ok(UnlockView {
                accepted: true,
                attempt,
                cooldown_ms: cooldown_ms as i64,
            })
        }
        Ok(Outcome::UnlockRefused {
            need_ms,
            remaining_ms,
        }) => {
            ::log::info!("[exit] 剩余不足一次冷却：需 {need_ms} ms，剩 {remaining_ms} ms");
            Err("E_REMAIN_TOO_SHORT".to_string())
        }
        Ok(Outcome::Settled { result, session }) => {
            finish(&app, &runtime, result, &session);
            Ok(UnlockView {
                accepted: false,
                attempt: 0,
                cooldown_ms: 0,
            })
        }
        Ok(other) => {
            ::log::warn!("[exit] 解锁未生效：{other:?}");
            Err("E_STATE_ILLEGAL".to_string())
        }
        Err(err) => {
            ::log::warn!("[exit] 解锁被拒：{err:?}");
            Err(map_error(err))
        }
    }
}

/// Challenge 内提交一次选择（PRD F-5.4 / F-5.5）
#[tauri::command]
pub fn submit_choice(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    question_id: String,
    choice: usize,
) -> Result<ChoiceView, String> {
    let elapsed = elapsed_of(&runtime);

    let outcome = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;

        // 题号对不上说明这是上一次的陈旧点击，直接丢弃
        let current = machine
            .session()
            .and_then(|session| session.round.as_ref())
            .and_then(|round| round.current_id())
            .map(|id| id.to_string());
        if current.as_deref() != Some(question_id.as_str()) {
            return Err("E_INVALID_CHOICE".to_string());
        }

        // 全对且本次需要抄写时，把原文一起交给状态机（抽取在 Rust，前端不参与）
        let copy = if machine.copy_chars() > 0 {
            runtime
                .quotes
                .copy_line(machine.copy_chars(), &mut Rng::from_entropy())
                .map(CopyTask::new)
        } else {
            None
        };

        machine.transition(SessionEvent::SubmitChoice { choice, copy }, elapsed)
    };

    match outcome {
        Ok(Outcome::ChallengeAdvanced) => {
            publish_state(&app, &runtime);
            Ok(ChoiceView {
                accepted: true,
                correct: true,
            })
        }
        Ok(Outcome::Cool {
            attempt,
            cooldown_ms,
        }) => {
            publish_state(&app, &runtime);
            ::log::info!(
                "[exit] 答错：第 {attempt} 次，冷却 {} 分钟",
                cooldown_ms / 60_000
            );
            Ok(ChoiceView {
                accepted: true,
                correct: false,
            })
        }
        Ok(Outcome::Copying { target_chars }) => {
            publish_state(&app, &runtime);
            ::log::info!("[exit] 全对，进入抄写（{target_chars} 字）");
            Ok(ChoiceView {
                accepted: true,
                correct: true,
            })
        }
        Ok(Outcome::Settled { result, session }) => {
            finish(&app, &runtime, result, &session);
            Ok(ChoiceView {
                accepted: true,
                correct: true,
            })
        }
        Ok(other) => {
            ::log::warn!("[exit] 答题未生效：{other:?}");
            Err("E_STATE_ILLEGAL".to_string())
        }
        Err(err) => Err(map_error(err)),
    }
}

/// Copying 内提交抄写内容（PRD F-5.8 / A-23）
#[tauri::command]
pub fn submit_copy(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    text: String,
) -> Result<CopyView, String> {
    let elapsed = elapsed_of(&runtime);
    let outcome = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;
        machine.transition(SessionEvent::SubmitCopy { text }, elapsed)
    };

    match outcome {
        Ok(Outcome::CopyMismatch { index }) => Ok(CopyView {
            ok: false,
            mismatch_index: Some(index),
        }),
        Ok(Outcome::Settled { result, session }) => {
            finish(&app, &runtime, result, &session);
            Ok(CopyView {
                ok: true,
                mismatch_index: None,
            })
        }
        Ok(other) => {
            ::log::warn!("[exit] 抄写未生效：{other:?}");
            Err("E_STATE_ILLEGAL".to_string())
        }
        Err(err) => Err(map_error(err)),
    }
}

/// Confirming → Settled(emergency)（PRD F-6.2）
#[tauri::command]
pub fn confirm_emergency(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    text: String,
) -> Result<(), String> {
    let elapsed = elapsed_of(&runtime);
    let outcome = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;
        machine.transition(SessionEvent::ConfirmEmergency { text }, elapsed)
    };

    match outcome {
        Ok(Outcome::Settled { result, session }) => {
            crate::platform::hook::esc_consume();
            finish(&app, &runtime, result, &session);
            Ok(())
        }
        Ok(other) => {
            ::log::warn!("[exit] 破戒确认未生效：{other:?}");
            Err("E_STATE_ILLEGAL".to_string())
        }
        Err(err) => Err(map_error(err)),
    }
}

/// Confirming → Locked（PRD F-6.4：不消耗成本，进度清零）
#[tauri::command]
pub fn cancel_emergency(app: AppHandle, runtime: State<'_, Arc<Runtime>>) -> Result<(), String> {
    let elapsed = elapsed_of(&runtime);
    let outcome = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;
        machine.transition(SessionEvent::CancelEmergency, elapsed)
    };

    match outcome {
        Ok(Outcome::Locked) => {
            // 取消时 Esc 往往还按着，必须清零并要求重新按下（否则下一帧又弹出来）
            crate::platform::hook::esc_consume();
            publish_state(&app, &runtime);
            ::log::info!("[exit] 已取消破戒");
            Ok(())
        }
        Ok(other) => {
            ::log::warn!("[exit] 取消破戒未生效：{other:?}");
            Err("E_STATE_ILLEGAL".to_string())
        }
        Err(err) => Err(map_error(err)),
    }
}
