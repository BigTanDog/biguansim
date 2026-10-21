//! 题库统计：记忆衰减的唯一数据源（PRD F-8.2 / §8 的 `bank/stats.json`）。
//!
//! 规则（2026-10-21 定稿）：
//! - **答对**过的题，30 天内不再被抽；
//! - **答错**的题，30 分钟内不再被抽（防止"错一题就知道答案"）；
//! - 冷却时长取决于**最近一次**作答结果，所以存 `lastCorrect`。
//!   PRD §8 的示例只有 `lastAskedMs/rightCount/wrongCount`，这里补一个字段：
//!   只靠计数推不出"最近这次是对是错"，会退化成"答对过的题永远 30 天冷却"。
//!
//! 文件损坏或丢失一律当作空表：按 PRD §7，`stats.json` 丢失只影响记忆衰减，可重建。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::store::Store;

pub const STATS_FILE: &str = "bank/stats.json";
/// 答对后的冷却：30 天
pub const RIGHT_COOLDOWN_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
/// 答错后的冷却：30 分钟
pub const WRONG_COOLDOWN_MS: i64 = 30 * 60 * 1_000;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Entry {
    pub last_asked_ms: Option<i64>,
    pub right_count: u32,
    pub wrong_count: u32,
    /// 最近一次作答是否正确：决定冷却窗口是 30 天还是 30 分钟
    pub last_correct: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(transparent)]
pub struct Table {
    entries: BTreeMap<String, Entry>,
}

impl Table {
    pub fn load(root: &Path) -> Table {
        let path = root.join(STATS_FILE);
        if !path.exists() {
            return Table::default();
        }
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Table>(&bytes) {
                Ok(table) => table,
                Err(err) => {
                    ::log::warn!("[bank] stats.json 解析失败，按空表处理：{err}");
                    Table::default()
                }
            },
            Err(err) => {
                ::log::warn!("[bank] 读取 stats.json 失败，按空表处理：{err}");
                Table::default()
            }
        }
    }

    pub fn save(&self, store: &Store) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        store.atomic_write(&store.root().join(STATS_FILE), &bytes)
    }

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    pub fn last_asked_ms(&self, id: &str) -> Option<i64> {
        self.entries.get(id).and_then(|entry| entry.last_asked_ms)
    }

    /// 这道题现在是否处于冷却（不该被抽）
    pub fn is_cooling(&self, id: &str, now_ms: i64) -> bool {
        let Some(entry) = self.entries.get(id) else {
            return false;
        };
        let Some(last) = entry.last_asked_ms else {
            return false;
        };

        let window = if entry.last_correct {
            RIGHT_COOLDOWN_MS
        } else {
            WRONG_COOLDOWN_MS
        };
        now_ms.saturating_sub(last) < window
    }

    /// 记录一次作答：答对与答错**都**写 `lastAskedMs`（F-8.2）
    pub fn mark(&mut self, id: &str, now_ms: i64, correct: bool) {
        let entry = self.entries.entry(id.to_string()).or_default();
        entry.last_asked_ms = Some(now_ms);
        entry.last_correct = correct;
        if correct {
            entry.right_count = entry.right_count.saturating_add(1);
        } else {
            entry.wrong_count = entry.wrong_count.saturating_add(1);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_500_000_000;

    fn table_with(id: &str, last: i64, correct: bool) -> Table {
        let mut table = Table::default();
        let entry = table.entries.entry(id.to_string()).or_default();
        entry.last_asked_ms = Some(last);
        entry.last_correct = correct;
        if correct {
            entry.right_count = 1;
        } else {
            entry.wrong_count = 1;
        }
        table
    }

    #[test]
    fn answered_right_is_cooled_for_30_days() {
        let table = table_with("q001", NOW - 24 * 60 * 60 * 1_000, true);
        assert!(table.is_cooling("q001", NOW), "答对 1 天后仍在冷却");

        let stale = table_with("q001", NOW - 31 * 24 * 60 * 60 * 1_000, true);
        assert!(!stale.is_cooling("q001", NOW), "答对 31 天后应可再抽");
    }

    #[test]
    fn answered_wrong_is_cooled_for_30_minutes() {
        let table = table_with("q002", NOW - 10 * 60 * 1_000, false);
        assert!(table.is_cooling("q002", NOW), "答错 10 分钟后仍在冷却");

        let stale = table_with("q002", NOW - 31 * 60 * 1_000, false);
        assert!(!stale.is_cooling("q002", NOW), "答错 31 分钟后应可再抽");
    }

    /// 回归：只靠 rightCount/wrongCount 推不出"最近这次"，
    /// 曾出现过"答对过一次 → 之后答错也按 30 天冷却"的错法
    #[test]
    fn cooldown_follows_the_latest_answer() {
        let mut table = Table::default();
        table.mark("q003", NOW - 40 * 24 * 60 * 60 * 1_000, true);
        table.mark("q003", NOW - 5 * 60 * 1_000, false);

        assert!(table.is_cooling("q003", NOW));
        assert!(
            !table.is_cooling("q003", NOW + 26 * 60 * 1_000),
            "最近一次是答错，30 分钟后就该解冻，而不是 30 天"
        );
    }

    #[test]
    fn unknown_question_is_not_cooling() {
        assert!(!Table::default().is_cooling("q999", NOW));
    }

    #[test]
    fn mark_counts_both_ways() {
        let mut table = Table::default();
        table.mark("q004", NOW, true);
        table.mark("q004", NOW + 1, false);
        table.mark("q004", NOW + 2, false);

        let entry = table.entry("q004").expect("有记录");
        assert_eq!(entry.right_count, 1);
        assert_eq!(entry.wrong_count, 2);
        assert_eq!(entry.last_asked_ms, Some(NOW + 2));
        assert!(!entry.last_correct);
    }

    #[test]
    fn corrupt_file_becomes_empty_table() {
        let dir = std::env::temp_dir().join(format!("biguansim-stats-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bank")).unwrap();
        std::fs::write(dir.join(STATS_FILE), b"{ broken").unwrap();

        let table = Table::load(&dir);
        assert!(table.is_empty(), "坏文件按空表处理");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
