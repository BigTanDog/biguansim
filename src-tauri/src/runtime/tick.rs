//! tick 线程用到的一次性动作：冷却到点抽题、起势推 3 秒。
//!
//! 从这里拆出去的原因：`mod.rs` 已经接近 500 行（AGENTS §4 的硬线是 600），
//! 而这两个函数与 tick 循环的局部状态无关，只依赖 `Runtime` 与 `AppHandle`，
//! 挪出来之后 `mod.rs` 只剩"线程编排 + 装配"，读起来清楚得多。

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::core::challenge::Round;
use crate::core::cost;
use crate::core::rng::Rng;
use crate::core::session::{Outcome, SessionEvent, SessionState, ARMING_MS};
use crate::core::timer::{MonotonicClock, Timer, WallClock};
use crate::events::{ArmingTick, StateChanged};
use crate::io_queue::IoJob;
use crate::platform::time::{SystemMonotonic, SystemWall};
use crate::platform::{focus, overlay, sleep};
use crate::runtime::{build_snapshot, publish_state, session_of, transition, Runtime};

/// 冷却到点 → 抽题 → Challenge。抽不到题就留在 Cool 并提示（绝不变成"可逃脱"）。
///
/// `last_warn` 用于限频：抽不到题时 tick 每 250 ms 会再进来一次，
/// 不限频就会每秒弹 4 条 toast。
pub(super) fn begin_challenge(
    app: &AppHandle,
    runtime: &Arc<Runtime>,
    elapsed: u64,
    last_warn: &mut Option<Instant>,
) {
    let Some(session) = session_of(runtime) else {
        return;
    };

    let needed = cost::question_count(session.attempts, session.question_base);
    let bank = match runtime.bank.lock() {
        Ok(guard) => Arc::clone(&guard),
        Err(_) => return,
    };

    let mut rng = Rng::from_entropy();
    // 记忆衰减用墙钟毫秒（与 `lastAskedMs` 同口径），不掺单调时钟
    let now_ms = SystemWall.wall_ms();
    let table = runtime
        .stats
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let drawn = bank.draw_with(&table, needed as usize, now_ms, &mut rng);
    if drawn.is_empty() {
        ::log::error!("[bank] 抽不到题（题库为空），保持冷却等待重试");
        let should_warn = last_warn.map_or(true, |at| at.elapsed() >= Duration::from_secs(10));
        if should_warn {
            *last_warn = Some(Instant::now());
            let _ = app.emit(
                "notice://toast",
                crate::events::Toast {
                    level: "error",
                    text: "题库为空，无法出题；本次只得继续等待".to_string(),
                },
            );
        }
        return;
    }

    if drawn.len() < needed as usize {
        // M4 的题库只有 12 题、单轮最多 5 题，正常不会触发；
        // 一旦 M5 接入记忆衰减，这条就是"退化规则没生效"的告警。
        ::log::warn!(
            "[challenge] 可抽题不足：需要 {needed} 题，只抽到 {} 题（M5 需按 F-8.2 三级退化补齐）",
            drawn.len()
        );
    }

    let round = Round::prepare(drawn, session.attempts, &mut rng);
    match transition(runtime, SessionEvent::CoolFinished { round }, elapsed) {
        Ok(Outcome::Challenge) => {
            publish_state(app, runtime);
            ::log::info!(
                "[challenge] 第 {} 次尝试，出 {} 题",
                session.attempts,
                needed
            );
        }
        Ok(other) => ::log::warn!("[challenge] 未进入问阵：{other:?}"),
        Err(err) => ::log::warn!("[challenge] 进入问阵被拒：{err:?}"),
    }
}

/// 起势：每 250 ms 推一次剩余毫秒；走满 3 秒后进 Locked 并启动计时器。
pub(super) fn step_arming(app: &AppHandle, runtime: &Arc<Runtime>) {
    let mono_now = SystemMonotonic.unbiased_ms();

    let Some((armed_at, target_ms)) = ({
        let machine = match runtime.machine.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        machine
            .session()
            .map(|session| (session.armed_at_mono_ms, session.target_ms))
    }) else {
        return;
    };

    let elapsed_arm = mono_now.saturating_sub(armed_at);
    let remaining = ARMING_MS.saturating_sub(elapsed_arm);

    let _ = app.emit(
        "arming://tick",
        ArmingTick {
            remaining_ms: remaining as i64,
        },
    );

    if remaining > 0 {
        return;
    }

    // 起势结束：状态机出口 + 启动计时器 + 首写快照
    if !matches!(
        transition(runtime, SessionEvent::ArmFinished, 0),
        Ok(Outcome::Locked)
    ) {
        ::log::warn!("[tick] 起势结束时状态已不是 Arming，忽略");
        return;
    }

    if let Ok(mut guard) = runtime.timer.lock() {
        *guard = Some(Timer::start(SystemMonotonic, SystemWall));
    }

    let snapshot = build_snapshot(runtime, 0);
    let _ = runtime.io_tx.send(IoJob::Snapshot(snapshot));

    let session = session_of(runtime);
    let goal = session.as_ref().and_then(|item| item.goal.clone());
    let speaker = session.as_ref().map(|item| item.speaker.clone());

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: SessionState::Locked.as_str().to_string(),
            result: None,
            goal,
            elapsed_ms: 0,
            target_ms: target_ms as i64,
            payload: None,
            speaker,
        },
    );

    // 结界的三件套要在进入 Locked 后才生效：起势期间还允许 Esc 取消
    if let Some(window) = app.get_webview_window(overlay::MAIN_LABEL) {
        match window.hwnd() {
            Ok(hwnd) => {
                if let Err(err) = focus::start(hwnd.0 as isize) {
                    ::log::error!("[focus] 启动失败：{err}");
                }
            }
            Err(err) => ::log::error!("[focus] 取窗口句柄失败：{err}"),
        }
    }
    sleep::block();

    ::log::info!("[tick] 起势结束，进入闭关");
}
