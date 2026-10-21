//! 状态投影的唯一构造处（TDD §6.3：状态与投影分离）。
//!
//! 为什么单独一个文件：五种活跃态各有自己的 `payload`，但**推事件的时机**必须统一
//! ——否则某个分支忘了推，前端就停在上一帧（例如抄写提交后仍显示旧的错字位置）。
//! 这里只读状态机，不改状态。

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::core::session::{SessionInfo, SessionState, EMERGENCY_SENTENCE};
use crate::events::{PhasePayload, StateChanged};
use crate::platform::focus;
use crate::runtime::{read_timer, Runtime};

pub fn publish_state(app: &AppHandle, runtime: &Arc<Runtime>) {
    let elapsed = read_timer(runtime).map(|(elapsed, _)| elapsed).unwrap_or(0);

    let (state, session) = {
        let machine = match runtime.machine.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        (machine.state(), machine.session().cloned())
    };

    // 抄写与急事确认要打中文：让输入法拿到前台（否则候选窗会被抢焦点顶掉）
    match state {
        SessionState::Copying | SessionState::Confirming => focus::pause(),
        _ => focus::resume(),
    }

    let Some(session) = session else {
        // 已经结算：Settled 由 finish() 统一推送，这里不重复发
        return;
    };

    let payload = payload_for(state, &session, elapsed);

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: state.as_str().to_string(),
            result: None,
            goal: session.goal.clone(),
            elapsed_ms: elapsed as i64,
            target_ms: session.target_ms as i64,
            payload,
        },
    );
}

/// 前端重连/重绘用：一次性读出当前状态的投影（`get_state` 调用它）
pub fn current_payload(runtime: &Arc<Runtime>) -> Option<PhasePayload> {
    let elapsed = read_timer(runtime).map(|(elapsed, _)| elapsed).unwrap_or(0);
    let (state, session) = {
        let machine = runtime.machine.lock().ok()?;
        (machine.state(), machine.session().cloned()?)
    };
    payload_for(state, &session, elapsed)
}

fn payload_for(state: SessionState, session: &SessionInfo, elapsed: u64) -> Option<PhasePayload> {
    match state {
        SessionState::Cool => Some(PhasePayload::Cool {
            attempt: session.attempts,
            // 用会话里存的实数（debug 构建会缩放冷却，不能再按公式重算一遍）
            cooldown_ms: session.cool_ms as i64,
            remain_ms: session.cooldown_remain_ms(elapsed).unwrap_or(0) as i64,
        }),
        SessionState::Challenge => {
            let view = session.round.as_ref()?.view()?;
            Some(PhasePayload::Challenge {
                id: view.id,
                index: view.index,
                total: view.total,
                stem: view.stem,
                options: view.options,
                attempt: session.attempts,
            })
        }
        SessionState::Copying => {
            let task = session.copy.as_ref()?;
            Some(PhasePayload::Copy {
                text: task.text.clone(),
                chars: task.chars,
            })
        }
        SessionState::Confirming => Some(PhasePayload::Confirm {
            sentence: EMERGENCY_SENTENCE.to_string(),
            elapsed_ms: elapsed as i64,
            faults: session.faults,
        }),
        _ => None,
    }
}
