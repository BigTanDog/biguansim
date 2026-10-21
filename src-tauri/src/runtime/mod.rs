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
//!
//! 分工：`publish.rs` 只读状态机造投影；`finish.rs` 是唯一的出关善后入口。

mod finish;
mod publish;

pub use finish::finish;
pub use publish::{current_payload, publish_state};

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::core::bank::Bank;
use crate::core::challenge::Round;
use crate::core::cost;
use crate::core::quote::QuoteBook;
use crate::core::rng::Rng;
use crate::core::session::{Outcome, SessionEvent, SessionMachine, SessionState, ARMING_MS};
use crate::core::store::{Record, SessionResult, SessionSnapshot, Store};
use crate::core::timer::{MonotonicClock, Timer, WallClock};
use crate::events::{ArmingTick, Chime, CoolTick, EscHold, StateChanged, TimerTick};
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
/// Esc 长按触发破戒确认（PRD F-6.1）
const ESC_HOLD_TRIGGER_MS: u64 = 8_000;

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
    /// 题库：只在入关与抽题时读，不进 `SessionInfo`（后者每 250 ms 被 clone）
    pub bank: Mutex<Arc<Bank>>,
    pub quotes: Arc<QuoteBook>,
}

pub fn init(app: &AppHandle) -> Arc<Runtime> {
    let dir = plog::app_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        ::log::error!("[ipc] 创建数据目录失败：{err}");
    }
    let store = Store::new(&dir);

    let (io_tx, io_rx) = mpsc::channel::<IoJob>();

    let recovered = recover_previous(&store);
    let bank = Bank::load(store.root());
    let quotes = QuoteBook::load(store.root());
    ::log::info!(
        "[bank] 已加载题库 {} 题；语料抄写段 {} 段",
        bank.len(),
        quotes.copy_count()
    );

    spawn_io(store.clone(), io_rx);

    let runtime = Arc::new(Runtime {
        machine: Mutex::new(SessionMachine::new()),
        timer: Mutex::new(None),
        store,
        io_tx,
        recovered: Mutex::new(recovered),
        bank: Mutex::new(Arc::new(bank)),
        quotes: Arc::new(quotes),
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

    handle_settled(app, runtime, outcome, "强制结算");
}

/// 便于命令层复用的"结算就善后"收口
pub fn handle_settled(
    app: &AppHandle,
    runtime: &Arc<Runtime>,
    outcome: Result<Outcome, crate::core::session::SessionError>,
    label: &str,
) {
    match outcome {
        Ok(Outcome::Settled { result, session }) => finish(app, runtime, result, &session),
        Ok(Outcome::Unchanged) => {}
        Ok(other) => ::log::debug!("[ipc] {label}：{other:?}"),
        Err(err) => ::log::warn!("[ipc] {label}被拒：{err:?}"),
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
        let mut last_esc_hold = 0u64;

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
                    last_esc_hold = 0;
                    continue;
                }
                s if s.is_active() => {}
                _ => {
                    last_remaining = u64::MAX;
                    last_esc_hold = 0;
                    continue;
                }
            }

            // 先读 timer 并立刻释放 guard，再取 machine：锁顺序固定为 timer → machine
            let Some((elapsed, skew)) = read_timer(&runtime) else {
                continue;
            };
            let Some(session) = session_of(&runtime) else {
                continue;
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

            // 1) 到点 / 冷却到点：唯一实现处（transition 内部先判剩余）
            let outcome = transition(&runtime, SessionEvent::Tick, elapsed);
            match outcome {
                Ok(Outcome::Settled { result, session }) => {
                    finish(&app, &runtime, result, &session);
                    last_snapshot = Instant::now();
                    last_remaining = u64::MAX;
                    last_esc_hold = 0;
                    continue;
                }
                Ok(Outcome::CooldownElapsed) => {
                    begin_challenge(&app, &runtime, elapsed);
                    continue;
                }
                _ => {}
            }

            // 2) 状态专属心跳
            if state == SessionState::Locked {
                let held = hook::esc_hold_ms();
                if held > 0 || last_esc_hold > 0 {
                    let _ = app.emit(
                        "esc://hold",
                        EscHold {
                            held_ms: held as i64,
                        },
                    );
                }
                last_esc_hold = held;

                if held >= ESC_HOLD_TRIGGER_MS {
                    // 8 秒判定在 Rust：前端卡住也破得了戒（M4 计划 D-27）
                    let outcome = transition(&runtime, SessionEvent::RequestEmergency, elapsed);
                    hook::esc_consume();
                    last_esc_hold = 0;
                    if matches!(outcome, Ok(Outcome::Confirming)) {
                        publish_state(&app, &runtime);
                        ::log::info!("[tick] Esc 按住 {held} ms，进入破戒确认");
                    }
                }
            } else {
                last_esc_hold = 0;
            }

            if state == SessionState::Cool {
                let remain = session.cooldown_remain_ms(elapsed).unwrap_or(0);
                let _ = app.emit(
                    "cool://tick",
                    CoolTick {
                        remain_ms: remain as i64,
                    },
                );
            }

            // 3) 末 10 秒三声轻滴答（PRD F-7.2）：只在跨过阈值的那一次触发
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

            if last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
                last_snapshot = Instant::now();
                let _ = runtime
                    .io_tx
                    .send(IoJob::Snapshot(build_snapshot(&runtime, elapsed)));
            }
        }
    });
}

fn transition(
    runtime: &Arc<Runtime>,
    event: SessionEvent,
    elapsed: u64,
) -> Result<Outcome, crate::core::session::SessionError> {
    let mut machine = match runtime.machine.lock() {
        Ok(guard) => guard,
        Err(_) => return Err(crate::core::session::SessionError::StateIllegal),
    };
    machine.transition(event, elapsed)
}

fn session_of(runtime: &Arc<Runtime>) -> Option<crate::core::session::SessionInfo> {
    let machine = runtime.machine.lock().ok()?;
    machine.session().cloned()
}

/// 冷却到点 → 抽题 → Challenge。抽不到题就留在 Cool 并提示（绝不变成"可逃脱"）。
fn begin_challenge(app: &AppHandle, runtime: &Arc<Runtime>, elapsed: u64) {
    let Some(session) = session_of(runtime) else {
        return;
    };

    let needed = cost::question_count(session.attempts, session.question_base);
    let bank = match runtime.bank.lock() {
        Ok(guard) => Arc::clone(&guard),
        Err(_) => return,
    };

    let mut rng = Rng::from_entropy();
    let drawn = bank.draw(needed as usize, &mut rng);
    if drawn.is_empty() {
        ::log::error!("[bank] 抽不到题（题库为空），保持冷却等待重试");
        let _ = app.emit(
            "notice://toast",
            crate::events::Toast {
                level: "error",
                text: "题库为空，无法出题；本次只得继续等待".to_string(),
            },
        );
        return;
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

    let goal = session_of(runtime).and_then(|session| session.goal);

    let _ = app.emit(
        "state://changed",
        StateChanged {
            state: SessionState::Locked.as_str().to_string(),
            result: None,
            goal,
            elapsed_ms: 0,
            target_ms: target_ms as i64,
            payload: None,
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
pub(crate) fn read_timer(runtime: &Arc<Runtime>) -> Option<(u64, i64)> {
    let guard = runtime.timer.lock().ok()?;
    let timer = guard.as_ref()?;
    Some((timer.elapsed_ms(), timer.skew_min()))
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

    let (state, session) = {
        let machine = runtime.machine.lock().ok();
        match machine.as_ref() {
            Some(machine) => (machine.state(), machine.session().cloned()),
            None => (SessionState::Idle, None),
        }
    };

    // 快照里的冷却时刻只为便于人工排查；判定仍用单调 elapsed（M4 计划 §3.3）
    let cooldown_until_ms = session.as_ref().and_then(|session| {
        session
            .cooldown_remain_ms(elapsed)
            .map(|remain| SystemWall.wall_ms() + remain as i64)
    });

    SessionSnapshot {
        state: state.as_str().to_string(),
        target_ms: session.as_ref().map(|s| s.target_ms as i64).unwrap_or(0),
        elapsed_ms: elapsed as i64,
        mono_base,
        wall_base_ms,
        started_at_ms: session.as_ref().map(|s| s.started_at_ms).unwrap_or(0),
        attempts: session.as_ref().map(|s| s.attempts).unwrap_or(0),
        cooldown_until_ms,
        bank: session
            .as_ref()
            .map(|s| s.bank.clone())
            .unwrap_or_else(|| "basic".to_string()),
        goal: session.as_ref().and_then(|s| s.goal.clone()),
    }
}
