//! 出关善后：**所有**离开活跃状态的路径都必须走这里（TDD §6.2）。
//!
//! 顺序固定不可调换：停"锁定中"标志 → 卸载钩子 → 停抢焦点 → 解除睡眠阻止 →
//! 收结界 → 追加记录 → 入队删快照 → 复位计时器 → 钟鸣 → 推状态。
//!
//! 为什么必须单一入口：这类软件最常见的事故是"某个分支漏还原"，表现为
//! 屏幕留在暗色、电源请求残留、窗口永远置顶、快照永远不清。

use tauri::{AppHandle, Emitter, Manager};

use crate::core::session::{SessionInfo, SessionState};
use crate::core::store::{Record, SessionResult};
use crate::core::timer::WallClock;
use crate::events::{Chime, StateChanged};
use crate::io_queue::IoJob;
use crate::platform::time::SystemWall;
use crate::platform::{focus, hook, overlay, sleep};
use crate::runtime::Runtime;

/// 出关善后。不再停止 tick/io 线程：它们是进程级的，停掉会让第二次入关直接锁死。
pub fn finish(
    app: &AppHandle,
    runtime: &std::sync::Arc<Runtime>,
    result: SessionResult,
    session: &SessionInfo,
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
            payload: None,
            speaker: Some(session.speaker.clone()),
        },
    );

    ::log::info!("[ipc] 出关：{result:?}，本次 {} ms", session.elapsed_ms);
}
