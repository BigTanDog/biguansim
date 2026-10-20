//! 线程编排与出关善后。
//!
//! 线程职责（TDD §1.3）：
//! - tick：250 ms 节拍，只读时钟、只推事件，**不写盘**
//! - io：唯一写盘者，串行消费队列
//! - power：消费系统挂起/恢复事件，喂给计时器
//!
//! `teardown()` 顺序固定：先停 tick（切断新的快照来源）→ 再停 io（排空队列）
//! → 收起结界 → 同步追加记录 → 删除快照。顺序颠倒就会出现"删了又被写回"。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::core::session::{Outcome, SessionEvent, SessionMachine, SessionState};
use crate::core::store::{Record, SessionResult, SessionSnapshot, Store};
use crate::core::timer::Timer;
use crate::events::{StateChanged, TimerTick};
use crate::platform::log as plog;
use crate::platform::time::{SystemMonotonic, SystemWall};
use crate::platform::{overlay, power};

/// 快照落盘间隔（TDD §4.8）
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(15);
/// tick 节拍
const TICK_INTERVAL: Duration = Duration::from_millis(250);

pub enum IoJob {
    Snapshot(SessionSnapshot),
    Record(Box<Record>),
}

pub struct RecoveredInfo {
    pub actual_ms: i64,
    pub started_at_ms: i64,
}

pub struct Runtime {
    pub machine: Mutex<SessionMachine>,
    pub timer: Mutex<Option<Timer<SystemMonotonic, SystemWall>>>,
    pub store: Store,
    pub io_tx: Sender<IoJob>,
    pub stop_tick: Arc<AtomicBool>,
    pub stop_io: Arc<AtomicBool>,
    pub io_done: Arc<AtomicBool>,
    pub recovered: Mutex<Option<RecoveredInfo>>,
}

pub fn init(app: &AppHandle) -> Arc<Runtime> {
    let dir = plog::app_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::error!("创建数据目录失败：{err}");
    }
    let store = Store::new(&dir);

    let (io_tx, io_rx) = mpsc::channel::<IoJob>();
    let stop_io = Arc::new(AtomicBool::new(false));
    let io_done = Arc::new(AtomicBool::new(false));
    let stop_tick = Arc::new(AtomicBool::new(false));

    let recovered = recover_previous(&store);

    spawn_io(store.clone(), io_rx, stop_io.clone(), io_done.clone());

    let runtime = Arc::new(Runtime {
        machine: Mutex::new(SessionMachine::new()),
        timer: Mutex::new(None),
        store,
        io_tx,
        stop_tick,
        stop_io,
        io_done,
        recovered: Mutex::new(recovered),
    });

    spawn_power(runtime.clone());
    spawn_tick(app.clone(), runtime.clone());

    runtime
}

/// 启动时发现残留快照 → 说明上次是崩溃/被杀。不续锁，记一条 `interrupted`。
fn recover_previous(store: &Store) -> Option<RecoveredInfo> {
    let snapshot = store.load_session()?;

    let record = Record {
        id: format!("s_{}", snapshot.started_at_ms),
        started_at_ms: snapshot.started_at_ms,
        ended_at_ms: snapshot.started_at_ms + snapshot.elapsed_ms,
        planned_ms: snapshot.target_ms,
        // PRD F-9.4：异常中断的时长是真实发生的，照常计入累计（不计有效日）
        actual_ms: snapshot.elapsed_ms,
        result: SessionResult::Interrupted,
        attempts: snapshot.attempts,
        faults: 0,
        bank: snapshot.bank.clone(),
    };

    if let Err(err) = store.append_jsonl(&store.records_path(), &record) {
        log::error!("写入 interrupted 记录失败：{err}");
    }
    if let Err(err) = store.clear_session() {
        log::error!("清理残留快照失败：{err}");
    }

    log::warn!(
        "检测到上次异常中断，已记入记录（{} ms），本次不续锁",
        snapshot.elapsed_ms
    );

    Some(RecoveredInfo {
        actual_ms: snapshot.elapsed_ms,
        started_at_ms: snapshot.started_at_ms,
    })
}

fn spawn_io(store: Store, rx: Receiver<IoJob>, stop: Arc<AtomicBool>, done: Arc<AtomicBool>) {
    thread::spawn(move || {
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(IoJob::Snapshot(snapshot)) => {
                    if let Err(err) = store.save_session(&snapshot) {
                        log::error!("写快照失败：{err}");
                    }
                }
                Ok(IoJob::Record(record)) => {
                    if let Err(err) = store.append_jsonl(&store.records_path(), &record) {
                        log::error!("写记录失败：{err}");
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        done.store(true, Ordering::SeqCst);
        log::info!("io 线程已退出");
    });
}

/// 消费电源事件。计时器只在 Locked 期间存在，其余时间忽略。
fn spawn_power(runtime: Arc<Runtime>) {
    let rx = power::watch();

    thread::spawn(move || {
        for event in rx {
            let mut guard = match runtime.timer.lock() {
                Ok(guard) => guard,
                Err(_) => continue,
            };

            if let Some(timer) = guard.as_mut() {
                match event.kind {
                    power::PowerEventKind::Suspend => {
                        timer.on_suspend();
                        log::info!("系统挂起，已记录挂起时刻");
                    }
                    power::PowerEventKind::Resume => {
                        timer.on_resume();
                        log::info!("系统恢复，累计已坚持 {} ms", timer.elapsed_ms());
                    }
                }
            }
        }
    });
}

fn spawn_tick(app: AppHandle, runtime: Arc<Runtime>) {
    let stop = runtime.stop_tick.clone();

    thread::spawn(move || {
        let mut last_snapshot = Instant::now();

        while !stop.load(Ordering::SeqCst) {
            thread::sleep(TICK_INTERVAL);

            // 只在锁定中推进；其余状态没有会话可读
            let reading = {
                let timer_guard = match runtime.timer.lock() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                let Some(timer) = timer_guard.as_ref() else {
                    continue;
                };

                let machine = match runtime.machine.lock() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                if machine.state() != SessionState::Locked {
                    continue;
                }
                let Some(session) = machine.session().cloned() else {
                    continue;
                };

                (timer.elapsed_ms(), session, timer.skew_min())
            };

            let (elapsed, _session, skew) = reading;
            let remaining = _session.target_ms.saturating_sub(elapsed);

            let _ = app.emit(
                "timer://tick",
                TimerTick {
                    elapsed_ms: elapsed as i64,
                    remaining_ms: remaining as i64,
                    clock_skew_min: skew,
                },
            );

            // 到点结算的唯一实现处（core::session::transition 内部先判剩余）
            let outcome = {
                let mut machine = match runtime.machine.lock() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                machine.transition(
                    SessionEvent::Tick {
                        elapsed_ms: elapsed,
                    },
                    elapsed,
                )
            };

            if let Ok(Outcome::Settled { result, session }) = outcome {
                finish(&app, &runtime, result, &session);
                break;
            }

            if last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
                last_snapshot = Instant::now();
                let _ = runtime
                    .io_tx
                    .send(IoJob::Snapshot(build_snapshot(&runtime, elapsed)));
            }
        }
    });
}

/// 出关善后。顺序不可调换（TDD §6.2 + M1 计划 D-9）。
pub fn finish(
    app: &AppHandle,
    runtime: &Arc<Runtime>,
    result: SessionResult,
    session: &crate::core::session::SessionInfo,
) {
    runtime.stop_tick.store(true, Ordering::SeqCst);

    runtime.stop_io.store(true, Ordering::SeqCst);
    for _ in 0..20 {
        if runtime.io_done.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    overlay::unlock_screen(app);

    let record = Record {
        id: session.id.clone(),
        started_at_ms: session.started_at_ms,
        ended_at_ms: session.started_at_ms + session.elapsed_ms as i64,
        planned_ms: session.target_ms as i64,
        actual_ms: session.elapsed_ms as i64,
        result,
        attempts: session.attempts,
        faults: session.faults,
        bank: session.bank.clone(),
    };

    if let Err(err) = runtime
        .store
        .append_jsonl(&runtime.store.records_path(), &record)
    {
        log::error!("写记录失败（E_STORE_FAILED）：{err}");
        let _ = app.emit(
            "notice://toast",
            crate::events::Toast {
                level: "error",
                text: "记录未能保存".to_string(),
            },
        );
    }

    if let Err(err) = runtime.store.clear_session() {
        log::error!("删除快照失败：{err}");
    }

    if let Ok(mut timer) = runtime.timer.lock() {
        *timer = None;
    }

    if let Some(state) = app.try_state::<crate::app_state::AppState>() {
        state.set_locked(false);
    }

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: SessionState::Settled.as_str().to_string(),
            result: Some(result),
            goal: session.goal.clone(),
            elapsed_ms: session.elapsed_ms as i64,
            target_ms: session.target_ms as i64,
        },
    );

    log::info!("出关：{result:?}，本次 {} ms", session.elapsed_ms);
}

pub fn build_snapshot(runtime: &Arc<Runtime>, elapsed: u64) -> SessionSnapshot {
    let machine = runtime.machine.lock().ok();
    let session = machine.as_ref().and_then(|m| m.session().cloned());

    let (mono_base, wall_base_ms) = match runtime.timer.lock() {
        Ok(timer) => match timer.as_ref() {
            Some(t) => (t.mono_base(), t.wall_base_ms()),
            None => (0, 0),
        },
        Err(_) => (0, 0),
    };

    SessionSnapshot {
        state: SessionState::Locked.as_str().to_string(),
        target_ms: session.as_ref().map(|s| s.target_ms as i64).unwrap_or(0),
        elapsed_ms: elapsed as i64,
        mono_base,
        wall_base_ms,
        started_at_ms: session.as_ref().map(|s| s.started_at_ms).unwrap_or(0),
        attempts: session.as_ref().map(|s| s.attempts).unwrap_or(0),
        cooldown_until_ms: None,
        bank: session
            .as_ref()
            .map(|s| s.bank.clone())
            .unwrap_or_else(|| "basic".to_string()),
        goal: session.as_ref().and_then(|s| s.goal.clone()),
    }
}
