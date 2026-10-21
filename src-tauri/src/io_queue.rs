//! 落盘队列：唯一的写盘者。
//!
//! 为什么单独拎出来：M1 的 P0（第二次入关锁死）根因就在"结束线程 vs 排队删除"的边界上。
//! 抽成取 `Receiver` 的纯函数后，"快照 → 删除"这个顺序保证可以单测，
//! 不必靠两次人工连跑。

use std::sync::mpsc::Receiver;

use crate::core::bank_stats::Table;
use crate::core::store::{SessionSnapshot, Store};

pub enum IoJob {
    Snapshot(SessionSnapshot),
    /// 正常出关：入队保证排在所有快照之后，天然不会"删了又被写回"
    ClearSnapshot,
    /// 答题后落 `bank/stats.json`（记忆衰减的唯一数据源）。
    /// 传整表而不是增量：文件很小（30 题），整表写更简单也不会写歪。
    BankStats(Table),
}

/// 消费队列直到发送端关闭（进程退出）。IO 线程只跑这一个循环。
pub fn run(store: &Store, rx: Receiver<IoJob>) {
    for job in rx {
        match job {
            IoJob::Snapshot(snapshot) => {
                if let Err(err) = store.save_session(&snapshot) {
                    ::log::error!("[io] 写快照失败：{err}");
                }
            }
            IoJob::ClearSnapshot => {
                if let Err(err) = store.clear_session() {
                    ::log::error!("[io] 删除快照失败：{err}");
                }
            }
            IoJob::BankStats(table) => {
                if let Err(err) = table.save(store) {
                    ::log::error!("[io] 写题库统计失败：{err}");
                }
            }
        }
    }

    ::log::info!("[io] 队列已关闭，线程退出");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "biguansim-io-{label}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("创建临时目录");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn snapshot() -> SessionSnapshot {
        SessionSnapshot {
            state: "Locked".to_string(),
            target_ms: 60_000,
            elapsed_ms: 15_000,
            mono_base: 1,
            wall_base_ms: 1_760_875_920_000,
            started_at_ms: 1_760_875_920_000,
            attempts: 0,
            cooldown_until_ms: None,
            bank: "basic".to_string(),
            goal: None,
        }
    }

    /// 回归（M1 P0）：快照之后删除，文件必须真的不在了，且不会再被写回
    #[test]
    fn clear_after_snapshot_leaves_no_file() {
        let dir = TempDir::new("clear");
        let store = Store::new(&dir.path);
        let (tx, rx) = mpsc::channel();

        let worker = {
            let store = store.clone();
            thread::spawn(move || run(&store, rx))
        };

        tx.send(IoJob::Snapshot(snapshot())).expect("入队快照");
        tx.send(IoJob::ClearSnapshot).expect("入队删除");
        drop(tx);
        worker.join().expect("io 线程正常退出");

        assert!(
            !store.session_path().exists(),
            "删除之后不得残留 session.json"
        );
    }

    /// 顺序即语义：删除之后又来的快照必须重新落盘（新会话或残留队列都要能识别）
    #[test]
    fn snapshot_after_clear_is_written() {
        let dir = TempDir::new("recreate");
        let store = Store::new(&dir.path);
        let (tx, rx) = mpsc::channel();

        let worker = {
            let store = store.clone();
            thread::spawn(move || run(&store, rx))
        };

        tx.send(IoJob::ClearSnapshot).expect("入队删除");
        tx.send(IoJob::Snapshot(snapshot())).expect("入队快照");
        drop(tx);
        worker.join().expect("io 线程正常退出");

        assert!(store.session_path().exists(), "删除之后的快照应重新落盘");
        assert!(store.load_session().is_some(), "落盘内容应可解析");
    }
}
