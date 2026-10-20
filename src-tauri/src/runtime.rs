//! 线程编排与出关善后。
//!
//! 线程职责（TDD §1.3）：
//! - tick：250 ms 节拍，只读时钟、只推事件，**不写盘**
//! - io：唯一写盘者，串行消费队列
//! - power：消费系统挂起/恢复事件，喂给计时器
//!
//! **tick 与 io 是进程级线程，不随单次闭关结束**。第一版把它们在 `finish()` 里停掉，
//! 结果第二次入关没有 tick 线程，倒计时冻结、永不结算、窗口又关不掉——直接违反
//! "绝不锁死"。现在它们是常驻线程：空闲时 tick 自己跳过，io 靠队列有序保证
//! "先写快照、后删快照"，不需要任何轮询等待。
//!
//! 锁顺序全项目统一（否则会 ABBA 死锁）：**先 timer、后 machine**，
//! 且 timer 的 guard 必须在取 machine 之前 drop。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::core::session::{Outcome, SessionEvent, SessionMachine, SessionState, ARMING_MS};
use crate::core::store::{Record, SessionResult, SessionSnapshot, Store};
use crate::core::timer::{MonotonicClock, Timer, WallClock};
use crate::events::{ArmingTick, Chime, StateChanged, TimerTick};
use crate::io_queue::IoJob;
use crate::platform::log as plog;
use crate::platform::time::{SystemMonotonic, SystemWall};
use crate::platform::{display, focus, hook, overlay, power, sleep};

/// 快照落盘间隔（TDD §4.8）
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(15);
/// tick 节拍
const TICK_INTERVAL: Duration = Duration::from_millis(250);
/// 末 10 秒三声轻滴答的触发点（PRD F-7.2）
const CHIME_THRESHOLDS: [u64; 3] = [10_000, 7_000, 4_000];

pub struct RecoveredInfo {
    pub actual_ms: i64,
    pub started_at_ms: i64,
}

pub struct Runtime {
    pub machine: Mutex<SessionMachine>,
    pub timer: Mutex<Option<Timer<SystemMonotonic, SystemWall>>>,
    pub store: Store,
    pub io_tx: Sender<IoJob>,
    pub recovered: Mutex<Option<RecoveredInfo>>,
}

pub fn init(app: &AppHandle) -> Arc<Runtime> {
    let dir = plog::app_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        ::log::error!("[ipc] 创建数据目录失败：{err}");
    }
    let store = Store::new(&dir);

    let (io_tx, io_rx) = mpsc::channel::<IoJob>();

    let recovered = recover_previous(&store);

    spawn_io(store.clone(), io_rx);

    let runtime = Arc::new(Runtime {
        machine: Mutex::new(SessionMachine::new()),
        timer: Mutex::new(None),
        store,
        io_tx,
        recovered: Mutex::new(recovered),
    });

    spawn_power(runtime.clone());
    display::spawn(app.clone(), runtime.clone());
    spawn_tick(app.clone(), runtime.clone());

    runtime
}

/// 强制结算（显示变化等外部事件用）。判定仍然只走 `transition`。
pub fn force_settle(app: &AppHandle, runtime: &Arc<Runtime>, result: SessionResult) {
    let elapsed = read_timer(runtime).map(|(elapsed, _)| elapsed).unwrap_or(0);

    let outcome = {
        let mut machine = match runtime.machine.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        // PRD §5.2：显示变化出关要记一次 fault
        if matches!(result, SessionResult::Accident) {
            machine.note_fault();
        }
        machine.transition(SessionEvent::Finish { result }, elapsed)
    };

    match outcome {
        Ok(Outcome::Settled { result, session }) => finish(app, runtime, result, &session),
        Ok(other) => ::log::warn!("[ipc] 强制结算未生效：{other:?}"),
        Err(err) => ::log::warn!("[ipc] 强制结算被拒：{err:?}"),
    }
}

/// 启动时发现残留快照 → 说明上次是崩溃/被杀。不续锁，记一条 `interrupted`。
fn recover_previous(store: &Store) -> Option<RecoveredInfo> {
    let snapshot = match store.load_session() {
        Some(snapshot) => snapshot,
        None => {
            // 文件存在却解析不出来：坏文件必须清掉，否则会永远留着
            if store.session_path().exists() && store.drop_corrupt_session() {
                ::log::warn!("[ipc] session.json 已损坏，已删除（不记中断）");
            }
            return None;
        }
    };

    let record = Record {
        id: format!("s_{}", snapshot.started_at_ms),
        started_at_ms: snapshot.started_at_ms,
        // 崩溃无法得知真实结束时刻，用快照里的已坚持时长推算（粒度 15 s）
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
        ::log::error!("[ipc] 写入 interrupted 记录失败：{err}");
    }
    if let Err(err) = store.clear_session() {
        ::log::error!("[ipc] 清理残留快照失败：{err}");
    }

    ::log::warn!(
        "[ipc] 检测到上次异常中断，已记入记录（{} ms），本次不续锁",
        snapshot.elapsed_ms
    );

    Some(RecoveredInfo {
        actual_ms: snapshot.elapsed_ms,
        started_at_ms: snapshot.started_at_ms,
    })
}

/// 常驻写盘线程：唯一写盘者。队列关闭（进程退出）时自然结束。
/// 具体的消费循环在 `io_queue::run`，抽出去是为了能单测"先写后删"的顺序保证。
fn spawn_io(store: Store, rx: Receiver<IoJob>) {
    thread::spawn(move || {
        crate::io_queue::run(&store, rx);
    });
}

/// 消费电源事件。**无论是否在闭关都要落日志**——M1 的验收要求就是"事件可见"。
fn spawn_power(runtime: Arc<Runtime>) {
    let rx = power::watch();

    thread::spawn(move || {
        for event in rx {
            let mut guard = match runtime.timer.lock() {
                Ok(guard) => guard,
                Err(_) => continue,
            };

            match event.kind {
                power::PowerEventKind::Suspend => {
                    if let Some(timer) = guard.as_mut() {
                        timer.on_suspend();
                        ::log::info!("[power] 系统挂起，已记录挂起时刻");
                    } else {
                        ::log::info!("[power] 系统挂起（当前未闭关，忽略计时）");
                    }
                }
                power::PowerEventKind::Resume => {
                    if let Some(timer) = guard.as_mut() {
                        timer.on_resume();
                        ::log::info!("[power] 系统恢复，累计已坚持 {} ms", timer.elapsed_ms());
                    } else {
                        ::log::info!("[power] 系统恢复（当前未闭关，忽略计时）");
                    }
                }
            }
        }
    });
}

fn spawn_tick(app: AppHandle, runtime: Arc<Runtime>) {
    thread::spawn(move || {
        let mut last_snapshot = Instant::now();
        let mut last_remaining = u64::MAX;

        loop {
            thread::sleep(TICK_INTERVAL);

            let state = match runtime.machine.lock() {
                Ok(machine) => machine.state(),
                Err(_) => continue,
            };

            match state {
                SessionState::Arming => {
                    step_arming(&app, &runtime);
                    last_remaining = u64::MAX;
                    continue;
                }
                SessionState::Locked => {}
                _ => {
                    last_remaining = u64::MAX;
                    continue;
                }
            }

            // 先读 timer 并立刻释放 guard，再取 machine：锁顺序固定为 timer → machine
            let Some((elapsed, skew)) = read_timer(&runtime) else {
                continue;
            };

            let session = {
                let machine = match runtime.machine.lock() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                match machine.session().cloned() {
                    Some(session) => session,
                    None => continue,
                }
            };

            let remaining = session.target_ms.saturating_sub(elapsed);

            let _ = app.emit(
                "timer://tick",
                TimerTick {
                    elapsed_ms: elapsed as i64,
                    remaining_ms: remaining as i64,
                    clock_skew_min: skew,
                },
            );

            // 末 10 秒三声轻滴答（PRD F-7.2）：只在跨过阈值的那一次触发
            if last_remaining != u64::MAX {
                for (index, threshold) in CHIME_THRESHOLDS.iter().enumerate() {
                    if last_remaining > *threshold && remaining <= *threshold {
                        let _ = app.emit(
                            "chime://play",
                            Chime {
                                kind: "tick",
                                index: index as u32,
                            },
                        );
                    }
                }
            }
            last_remaining = remaining;

            // 到点结算的唯一实现处（core::session::transition 内部先判剩余）
            let outcome = {
                let mut machine = match runtime.machine.lock() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                machine.transition(SessionEvent::Tick, elapsed)
            };

            if let Ok(Outcome::Settled { result, session }) = outcome {
                finish(&app, &runtime, result, &session);
                last_snapshot = Instant::now();
                last_remaining = u64::MAX;
                continue;
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

/// 起势：每 250 ms 推一次剩余毫秒；走满 3 秒后进 Locked 并启动计时器。
fn step_arming(app: &AppHandle, runtime: &Arc<Runtime>) {
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
    let outcome = {
        let mut machine = match runtime.machine.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        machine.transition(SessionEvent::ArmFinished, 0)
    };

    if !matches!(outcome, Ok(Outcome::Locked)) {
        ::log::warn!("[tick] 起势结束时状态已不是 Arming，忽略");
        return;
    }

    if let Ok(mut guard) = runtime.timer.lock() {
        *guard = Some(Timer::start(SystemMonotonic, SystemWall));
    }

    let snapshot = build_snapshot(runtime, 0);
    let _ = runtime.io_tx.send(IoJob::Snapshot(snapshot));

    let goal = {
        let machine = match runtime.machine.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        machine.session().and_then(|session| session.goal.clone())
    };

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

/// 读当前已坚持时长与时间偏差；没有会话（timer 为 None）时返回 None。
fn read_timer(runtime: &Arc<Runtime>) -> Option<(u64, i64)> {
    let guard = runtime.timer.lock().ok()?;
    let timer = guard.as_ref()?;
    Some((timer.elapsed_ms(), timer.skew_min()))
}

/// 出关善后。不再停止 tick/io 线程：它们是进程级的，停掉会让第二次入关直接锁死。
pub fn finish(
    app: &AppHandle,
    runtime: &Arc<Runtime>,
    result: SessionResult,
    session: &crate::core::session::SessionInfo,
) {
    // 先掐掉"锁定中"标志，让置顶维持线程立刻退出；
    // 否则它会在 unlock 之后又把主窗口重新置顶
    if let Some(state) = app.try_state::<crate::app_state::AppState>() {
        state.set_locked(false);
    }

    hook::uninstall();
    // 先停抢焦点再解锁，否则它会在窗口恢复标题栏的过程中继续抢
    focus::stop();
    sleep::allow();
    overlay::unlock_screen(app);

    let record = Record {
        id: session.id.clone(),
        started_at_ms: session.started_at_ms,
        // 真实结束时刻用墙钟取，不能用 started + elapsed：后者在被改过时间后是错的
        ended_at_ms: SystemWall.wall_ms(),
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
        ::log::error!("[ipc] 写记录失败（E_STORE_FAILED）：{err}");
        let _ = app.emit(
            "notice://toast",
            crate::events::Toast {
                level: "error",
                text: "记录未能保存".to_string(),
            },
        );
    }

    // 入队而不是同步删：队列里排在所有快照之后，慢盘也不会"删了又被写回"
    if runtime.io_tx.send(IoJob::ClearSnapshot).is_err() {
        ::log::error!("[ipc] io 队列已关闭，快照未能清理");
    }

    if let Ok(mut timer) = runtime.timer.lock() {
        *timer = None;
    }

    // 到点钟鸣（PRD F-7.2）：一次事件，前端连响 3 声
    let _ = app.emit(
        "chime://play",
        Chime {
            kind: "bell",
            index: 0,
        },
    );

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

    ::log::info!("[ipc] 出关：{result:?}，本次 {} ms", session.elapsed_ms);
}

pub fn build_snapshot(runtime: &Arc<Runtime>, elapsed: u64) -> SessionSnapshot {
    // 锁顺序：先 timer，取完即释放
    let (mono_base, wall_base_ms) = {
        let guard = runtime.timer.lock().ok();
        match guard.as_ref().and_then(|g| g.as_ref()) {
            Some(timer) => (timer.mono_base(), timer.wall_base_ms()),
            None => (0, 0),
        }
    };

    let session = {
        let machine = runtime.machine.lock().ok();
        machine.as_ref().and_then(|m| m.session().cloned())
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
