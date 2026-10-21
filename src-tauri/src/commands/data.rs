//! 记录 / 题库 / 配置三个只读或半只读入口（PRD F-9 / F-8.6 / F-10）。
//!
//! 这里只做"取数 + 校验 + 落盘"，所有判定与口径都在 `core::stats`，
//! 命令层不自己算日期、不算连续天数（否则两处口径迟早分叉）。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::core::store::{Config, Record};
use crate::core::timer::WallClock;
use crate::platform::time::{local_date, SystemWall};
use crate::runtime::Runtime;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    total_ms: i64,
    sessions: u32,
    faults: u32,
    valid_days: u32,
    streak_days: u32,
    today_valid: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordView {
    started_at_ms: i64,
    actual_ms: i64,
    planned_ms: i64,
    result: String,
    attempts: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordPage {
    records: Vec<RecordView>,
    /// `records.jsonl` 里跳过的损坏行数（PRD §12：逐行容错，坏行跳过并在界面提示）
    skipped: usize,
    stats: StatsView,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionSummary {
    id: String,
    stem: String,
    tags: Vec<String>,
    right_count: u32,
    wrong_count: u32,
    cooling: bool,
}

/// 把记录换算成本地日并交给纯函数统计（`core::stats`）
pub fn stats_view(runtime: &Arc<Runtime>) -> StatsView {
    let (records, _) = runtime.store.load_records().unwrap_or_default();
    let dated: Vec<crate::core::stats::DatedRecord> = records
        .iter()
        .map(|record| crate::core::stats::DatedRecord {
            date: local_date(record.started_at_ms),
            result: record.result,
            actual_ms: record.actual_ms,
        })
        .collect();

    let today = local_date(SystemWall.wall_ms());
    let summary = crate::core::stats::summary(&dated, today);

    StatsView {
        total_ms: summary.total_ms,
        sessions: summary.sessions,
        faults: summary.faults,
        valid_days: summary.valid_days,
        streak_days: summary.streak_days,
        today_valid: summary.today_valid,
    }
}

/// 记录页：最近 `limit` 条（按开始时间倒序）。
#[tauri::command]
pub fn list_records(runtime: State<'_, Arc<Runtime>>, limit: usize) -> RecordPage {
    let (mut records, skipped) = runtime.store.load_records().unwrap_or_default();
    records.sort_by_key(|record| std::cmp::Reverse(record.started_at_ms));

    let limit = limit.clamp(1, 200);
    let records = records
        .iter()
        .take(limit)
        .map(|record: &Record| RecordView {
            started_at_ms: record.started_at_ms,
            actual_ms: record.actual_ms,
            planned_ms: record.planned_ms,
            result: match record.result {
                crate::core::store::SessionResult::Completed => "completed",
                crate::core::store::SessionResult::Early => "early",
                crate::core::store::SessionResult::Emergency => "emergency",
                crate::core::store::SessionResult::Accident => "accident",
                crate::core::store::SessionResult::Interrupted => "interrupted",
            }
            .to_string(),
            attempts: record.attempts,
        })
        .collect();

    RecordPage {
        records,
        skipped,
        stats: stats_view(&runtime),
    }
}

/// 题库页：只给题面与统计，**不给答案**（F-8.6）
#[tauri::command]
pub fn list_questions(runtime: State<'_, Arc<Runtime>>) -> Vec<QuestionSummary> {
    let bank = match runtime.bank.lock() {
        Ok(guard) => Arc::clone(&guard),
        Err(_) => return Vec::new(),
    };
    let table = runtime
        .stats
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let now_ms = SystemWall.wall_ms();

    bank.questions
        .iter()
        .map(|question| {
            let entry = table.entry(&question.id);
            QuestionSummary {
                id: question.id.clone(),
                stem: question.stem.clone(),
                tags: question.tags.clone(),
                right_count: entry.map(|e| e.right_count).unwrap_or(0),
                wrong_count: entry.map(|e| e.wrong_count).unwrap_or(0),
                cooling: table.is_cooling(&question.id, now_ms),
            }
        })
        .collect()
}

/// 设置页：浅合并 `patch` → 校验 → 原子写。非法值**不落盘**。
#[tauri::command]
pub fn update_config(
    runtime: State<'_, Arc<Runtime>>,
    patch: serde_json::Value,
) -> Result<Config, String> {
    let current = runtime.store.load_config();

    let mut base =
        serde_json::to_value(&current).map_err(|err| format!("E_INVALID_CONFIG: {err}"))?;
    if let (Some(base_map), Some(patch_map)) = (base.as_object_mut(), patch.as_object()) {
        for (key, value) in patch_map {
            base_map.insert(key.clone(), value.clone());
        }
    }

    let next: Config =
        serde_json::from_value(base).map_err(|err| format!("E_INVALID_CONFIG: {err}"))?;
    validate(&next)?;

    runtime
        .store
        .save_config(&next)
        .map_err(|err| format!("E_STORE_FAILED: {err}"))?;

    ::log::info!("[config] 设置已更新并落盘");
    Ok(next)
}

/// 打开数据目录（设置页 F-10.8）
#[tauri::command]
pub fn open_data_dir(runtime: State<'_, Arc<Runtime>>) -> Result<(), String> {
    let root = runtime.store.root().to_path_buf();
    std::process::Command::new("explorer")
        .arg(&root)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("E_OPEN_DIR: {err}"))
}

fn validate(config: &Config) -> Result<(), String> {
    if !(3..=10).contains(&config.cooldown_base) {
        return Err("E_INVALID_CONFIG: 冷却基数须在 3–10 分钟".to_string());
    }
    if !(3..=5).contains(&config.question_base) {
        return Err("E_INVALID_CONFIG: 题数基数须在 3–5".to_string());
    }
    if !(0.0..=0.6).contains(&config.chime_volume) {
        return Err("E_INVALID_CONFIG: 音量须在 0–0.6".to_string());
    }
    if !matches!(config.theme.as_str(), "dark" | "warm") {
        return Err("E_INVALID_CONFIG: 主题只能是 dark 或 warm".to_string());
    }
    Ok(())
}
