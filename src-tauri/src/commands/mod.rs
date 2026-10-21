//! IPC 命令（TDD §5.1）。入关/取消/查询在这里，退出流程的五个命令在 `exit.rs`。

pub mod data;
pub mod exit;

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::app_state::AppState;
use crate::core::bank::Bank;
use crate::core::rng::Rng;
use crate::core::session::{Outcome, SessionEvent, SessionState};
use crate::core::store::Config;
use crate::core::timer::{MonotonicClock, WallClock};
use crate::events::{PhasePayload, StateChanged};
use crate::platform::time::{SystemMonotonic, SystemWall};
use crate::platform::{hook, overlay};
use crate::runtime::{current_payload, Runtime};

const MIN_DURATION_MIN: u32 = 10;
const MAX_DURATION_MIN: u32 = 480;
/// 入关的题库下限：少于这个数，单轮的蒙对率会从 1/64 跳到 1/16（F-5.2）
const MIN_QUESTIONS: usize = 3;

/// 说话人候选：每关随机一个，避免每次都像同一个人在同一处念同一套台词。
/// 「看守弟子」不在此列——冷却时固定由他去通报长老（F-4.3）。
const SPEAKERS: [&str; 4] = ["长老", "阵灵", "前辈", "冥冥之中的声音"];

fn pick_speaker() -> String {
    let mut rng = Rng::from_entropy();
    let index = rng.below(SPEAKERS.len() as u32) as usize;
    SPEAKERS.get(index).copied().unwrap_or("长老").to_string()
}

/// (题库总数, 当前可抽数)。可抽 = 不在记忆衰减冷却里的题（F-1.2 要求分开显示）。
fn bank_counts(runtime: &Arc<Runtime>) -> (usize, usize) {
    let Ok(bank) = runtime.bank.lock() else {
        return (0, 0);
    };

    let total = bank.len();
    let now_ms = SystemWall.wall_ms();
    let drawable = match runtime.stats.lock() {
        Ok(table) => bank
            .questions
            .iter()
            .filter(|question| !table.is_cooling(&question.id, now_ms))
            .count(),
        Err(_) => total,
    };

    (total, drawable)
}
/// 测试版把冷却缩到这个倍数（5 分钟 → 30 秒），否则自测要走完
/// 5+10+15 分钟的冷却才能到第 3 轮抄写，一次验证半小时。
/// 正式版恒为 1（这个常量只在 debug 编译里被用到）。
#[cfg(debug_assertions)]
const COST_SCALE: u32 = 10;
#[cfg(not(debug_assertions))]
const COST_SCALE: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSessionView {
    actual_ms: i64,
    started_at_ms: i64,
}

/// 题库概况：配置页要显示"可抽 X 题 / 共 Y 题"，且总数 < 30 时必须出黄字（F-1.2 / F-8.3）。
/// 数字由 Rust 给出，前端不写死。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BankView {
    id: String,
    name: String,
    total: usize,
    /// 当前**可抽**题数（去掉冷却中的）——F-1.2 要求与总数分开显示，
    /// 且"可抽很少"不算阵法简陋，只有总数 < 30 才出黄字（A-16）
    drawable: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapView {
    config: Config,
    bank: BankView,
    /// 累计 / 连续天数 / 破戒（F-1.4）
    stats: data::StatsView,
    /// 上次异常中断的会话（本次启动时已记入记录）；正常启动为 None
    last_session: Option<LastSessionView>,
    /// 是否 debug 构建：前端据此显示 DEV 退出按钮与"测试版"提示。
    /// 用 Rust 的 `cfg!` 而不是 Vite 的 `import.meta.env.DEV`——后者在
    /// `tauri build --debug` 里是 false，导致测试版没有逃生口（2026-10-21 踩过）。
    debug_build: bool,
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
    /// 活跃态投影（冷却/问阵/抄写/确认）；其余状态为 None
    payload: Option<PhasePayload>,
    speaker: Option<String>,
    bank_total: usize,
    bank_drawable: usize,
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

    let (total, drawable) = bank_counts(&runtime);
    let bank = match runtime.bank.lock() {
        Ok(bank) => BankView {
            id: bank.id.clone(),
            name: bank.name.clone(),
            total,
            drawable,
        },
        Err(_) => BankView {
            id: "basic".to_string(),
            name: "基础阵法".to_string(),
            total,
            drawable,
        },
    };

    BootstrapView {
        config,
        bank,
        last_session,
        stats: data::stats_view(&runtime),
        debug_build: cfg!(debug_assertions),
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
    let now_mono_ms = SystemMonotonic.unbiased_ms();
    let bank = bank_id.unwrap_or_else(|| "basic".to_string());
    // 每关换一个说话人（阵灵/前辈/冥冥之中的声音/长老），避免台词永远出自同一张嘴
    let speaker = pick_speaker();
    let speaker_label = speaker.clone();
    // 成本基数在入关时固化进会话：状态机因此不必读文件，也不会中途被改配置影响
    let mut config = runtime.store.load_config();

    // 题库在入关时重载一次（用户可能刚手动改过）。
    // **少于 3 题绝不允许入关**：F-5.2 的"蒙不过去"建立在"至少 3 题"上
    // （3 题全对 1/64，2 题就变成 1/16），题库被手改成 2 题时宁可拒绝入关。
    let loaded = Bank::load(runtime.store.root());
    if loaded.len() < MIN_QUESTIONS {
        log::error!(
            "[ipc] 题库只有 {} 题（下限 {MIN_QUESTIONS}），拒绝入关（E_BANK_INVALID）",
            loaded.len()
        );
        return Err(format!(
            "E_BANK_INVALID: 题库至少需要 {MIN_QUESTIONS} 题，当前 {} 题",
            loaded.len()
        ));
    }
    if let Ok(mut guard) = runtime.bank.lock() {
        *guard = Arc::new(loaded);
    }

    // Idle/Settled → Arming。真正进入 Locked 由 tick 线程在 3 秒后完成（PRD F-2.1）
    let (session_id, target_ms) = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;

        match machine.transition(
            SessionEvent::Start {
                duration_ms,
                now_ms,
                now_mono_ms,
                goal: goal.clone(),
                bank,
                cooldown_base: config.cooldown_base,
                question_base: config.question_base,
                cost_scale: COST_SCALE,
                speaker,
            },
            0,
        ) {
            Ok(Outcome::Arming) => {
                let session = machine
                    .session()
                    .ok_or_else(|| "E_STATE_ILLEGAL".to_string())?;
                (session.id.clone(), session.target_ms)
            }
            _ => return Err("E_STATE_ILLEGAL".to_string()),
        }
    };

    // 先装钩子：装不上就绝不进闭关（PRD D-7，绝不允许"看起来锁了其实没锁"）
    if let Err(err) = hook::install(&app) {
        if let Ok(mut machine) = runtime.machine.lock() {
            machine.reset();
        }
        log::error!("[ipc] 钩子安装失败，拒绝入关：{err}");
        return Err(format!("E_HOOK_FAILED: {err}"));
    }
    hook::reset_state();

    // 必须先置 locked 再展开结界：overlay 里的置顶维持线程靠这个标志判断存活，
    // 顺序颠倒的话它会先看到 false 然后直接退出，本次闭关就再也不会重申 TOPMOST。
    state.set_locked(true);

    if let Err(err) = overlay::lock_screen(&app) {
        // 主屏可能已经被改成无边框置顶，必须回滚，否则留下一个关不掉的大窗
        overlay::unlock_screen(&app);
        hook::uninstall();
        if let Ok(mut machine) = runtime.machine.lock() {
            machine.reset();
        }
        log::error!("[ipc] 入关失败，已回滚：{err}");
        return Err(format!("E_OVERLAY_FAILED: {err}"));
    }

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: SessionState::Arming.as_str().to_string(),
            result: None,
            goal,
            elapsed_ms: 0,
            target_ms: target_ms as i64,
            payload: None,
            speaker: Some(speaker_label),
        },
    );

    // 「再入一关」要预填上次时长（F-7.6）：把这次的时长写回 config
    if config.last_duration_min != duration_min {
        config.last_duration_min = duration_min;
        if let Err(err) = runtime.store.save_config(&config) {
            log::error!("[ipc] 保存 lastDurationMin 失败：{err}");
        }
    }

    log::info!("[ipc] 入关（起势）：{duration_min} 分钟，会话 {session_id}");
    Ok(StartResult {
        ok: true,
        session_id,
    })
}

/// 起势 3 秒内取消（PRD F-2.2）。非 Arming 状态一律 `E_STATE_ILLEGAL`。
#[tauri::command]
pub fn cancel_arming(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    let outcome = {
        let mut machine = runtime
            .machine
            .lock()
            .map_err(|_| "E_STATE_ILLEGAL".to_string())?;
        machine.transition(SessionEvent::Cancel, 0)
    };

    if !matches!(outcome, Ok(Outcome::Idle)) {
        return Err("E_STATE_ILLEGAL".to_string());
    }

    app_state.set_locked(false);
    hook::uninstall();
    overlay::unlock_screen(&app);

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: SessionState::Idle.as_str().to_string(),
            result: None,
            goal: None,
            elapsed_ms: 0,
            target_ms: 0,
            payload: None,
            speaker: None,
        },
    );

    log::info!("[ipc] 起势已取消");
    Ok(())
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
        payload: current_payload(&runtime),
        speaker: session.as_ref().map(|item| item.speaker.clone()),
        bank_total: bank_counts(&runtime).0,
        bank_drawable: bank_counts(&runtime).1,
    }
}

/// 开发期逃生口：**只在 debug 构建里存在**（release 编译期剔除）。
/// 本地测试时不必等闭关结束；不写记录、不计入统计。
#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_abort(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    if let Ok(mut machine) = runtime.machine.lock() {
        machine.reset();
    }
    if let Ok(mut timer) = runtime.timer.lock() {
        *timer = None;
    }

    app_state.set_locked(false);
    hook::uninstall();
    crate::platform::focus::stop();
    crate::platform::sleep::allow();
    overlay::unlock_screen(&app);
    let _ = runtime.io_tx.send(crate::io_queue::IoJob::ClearSnapshot);

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: SessionState::Idle.as_str().to_string(),
            result: None,
            goal: None,
            elapsed_ms: 0,
            target_ms: 0,
            payload: None,
            speaker: None,
        },
    );

    log::warn!("[dev] 手动退出（不写记录）");
    Ok(())
}
