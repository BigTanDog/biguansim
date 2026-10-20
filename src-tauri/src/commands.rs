//! IPC 命令（TDD §5.1）。M1 只实现三个：bootstrap / start_session / get_state，
//! M2 起只做加法。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_state::AppState;
use crate::core::session::{Outcome, SessionEvent, SessionState};
use crate::core::store::Config;
use crate::core::timer::{Timer, WallClock};
use crate::events::StateChanged;
use crate::platform::overlay;
use crate::platform::time::{SystemMonotonic, SystemWall};
use crate::runtime::Runtime;

const MIN_DURATION_MIN: u32 = 15;
const MAX_DURATION_MIN: u32 = 480;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSessionView {
    actual_ms: i64,
    started_at_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapView {
    config: Config,
    /// 上次异常中断的会话（本次启动时已记入记录）；正常启动为 None
    last_session: Option<LastSessionView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    ok: bool,
    session_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    state: String,
    target_ms: i64,
    elapsed_ms: i64,
    remaining_ms: i64,
    goal: Option<String>,
    bank: String,
    attempts: u32,
    clock_skew_min: i64,
}

#[tauri::command]
pub fn bootstrap(runtime: State<'_, Arc<Runtime>>) -> BootstrapView {
    let config = runtime.store.load_config();
    let last_session = runtime.recovered.lock().ok().and_then(|guard| {
        guard.as_ref().map(|info| LastSessionView {
            actual_ms: info.actual_ms,
            started_at_ms: info.started_at_ms,
        })
    });

    BootstrapView {
        config,
        last_session,
    }
}

#[tauri::command]
pub fn start_session(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    state: State<'_, AppState>,
    duration_min: u32,
    goal: Option<String>,
    bank_id: Option<String>,
) -> Result<StartResult, String> {
    if !(MIN_DURATION_MIN..=MAX_DURATION_MIN).contains(&duration_min) {
        return Err("E_INVALID_DURATION".to_string());
    }

    let duration_ms = duration_min as u64 * 60_000;
    let now_ms = SystemWall.wall_ms();
    let bank = bank_id.unwrap_or_else(|| "basic".to_string());

    let (session_id, target_ms) = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;

        match machine.transition(
            SessionEvent::Start {
                duration_ms,
                now_ms,
                goal: goal.clone(),
                bank,
            },
            0,
        ) {
            Ok(Outcome::Locked) => {
                let session = machine
                    .session()
                    .ok_or_else(|| "E_STATE_ILLEGAL".to_string())?;
                (session.id.clone(), session.target_ms)
            }
            _ => return Err("E_STATE_ILLEGAL".to_string()),
        }
    };

    // 结界起不来就不进闭关（PRD D-7：绝不允许"看起来锁了其实没锁"）
    if let Err(err) = overlay::lock_screen(&app) {
        // 主屏可能已经被改成无边框置顶，必须回滚，否则留下一个关不掉的大窗
        overlay::unlock_screen(&app);
        if let Ok(mut machine) = runtime.machine.lock() {
            machine.reset();
        }
        log::error!("[ipc] 入关失败，已回滚：{err}");
        return Err(format!("E_OVERLAY_FAILED: {err}"));
    }

    let timer = Timer::start(SystemMonotonic, SystemWall);
    if let Ok(mut guard) = runtime.timer.lock() {
        *guard = Some(timer);
    }

    state.set_locked(true);

    // 首写快照（PRD §5.2：进入 Locked 时写）。
    // 走 io 队列而不是直接写盘：io 线程是唯一写盘者，直接写会和它抢同一个 .tmp
    let snapshot = crate::runtime::build_snapshot(&runtime, 0);
    if runtime
        .io_tx
        .send(crate::runtime::IoJob::Snapshot(snapshot))
        .is_err()
    {
        log::error!("[ipc] io 队列不可用，首写快照失败");
    }

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: SessionState::Locked.as_str().to_string(),
            result: None,
            goal,
            elapsed_ms: 0,
            target_ms: target_ms as i64,
        },
    );

    log::info!("入关：{duration_min} 分钟，会话 {session_id}");
    Ok(StartResult {
        ok: true,
        session_id,
    })
}

#[tauri::command]
pub fn get_state(runtime: State<'_, Arc<Runtime>>) -> StateView {
    // 锁顺序固定为 timer → machine，且 timer 的 guard 在取 machine 之前释放
    let (elapsed, skew) = match runtime.timer.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(timer) => (timer.elapsed_ms(), timer.skew_min()),
            None => (0, 0),
        },
        Err(_) => (0, 0),
    };

    let (state, session) = {
        let machine = runtime.machine.lock().ok();
        match machine.as_ref() {
            Some(machine) => (machine.state(), machine.session().cloned()),
            None => (SessionState::Idle, None),
        }
    };

    StateView {
        state: state.as_str().to_string(),
        target_ms: session.as_ref().map(|s| s.target_ms as i64).unwrap_or(0),
        elapsed_ms: elapsed as i64,
        remaining_ms: session
            .as_ref()
            .map(|s| s.target_ms.saturating_sub(elapsed) as i64)
            .unwrap_or(0),
        goal: session.as_ref().and_then(|s| s.goal.clone()),
        bank: session
            .as_ref()
            .map(|s| s.bank.clone())
            .unwrap_or_else(|| "basic".to_string()),
        attempts: session.as_ref().map(|s| s.attempts).unwrap_or(0),
        clock_skew_min: skew,
    }
}
