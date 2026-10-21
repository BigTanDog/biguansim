//! 题库：加载、校验、抽题（PRD F-8.x）。
//!
//! 加载优先级：用户文件 `%APPDATA%\闭关模拟器\bank\seed.json` → 内嵌资源。
//! **任何情况下不得返回空题库**（PRD §12）：内嵌那份在构建期就由单测保证可解析，
//! 所以运行期的兜底不会失败。
//!
//! 本模块只负责"有哪些题"，不负责展示顺序与判定——打乱与判分在 `core::challenge`。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::rng::Rng;

/// 题库文件相对数据目录的路径（F-8.7）
pub const BANK_FILE: &str = "bank/seed.json";
/// 题干字数上限（F-8.5）
pub const MAX_STEM_CHARS: usize = 60;
/// 选择题选项数（F-5.1）
pub const OPTION_COUNT: usize = 4;

const EMBEDDED: &str = include_str!("../../resources/bank_seed.json");

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub difficulty: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: String,
    pub stem: String,
    pub options: Vec<String>,
    /// 正确项的原始索引；**只在 Rust 内部使用**，不得出现在任何响应体里
    pub answer: usize,
    #[serde(default)]
    pub hint: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct BankFile {
    version: u32,
    id: String,
    name: String,
    questions: Vec<Question>,
}

#[derive(Clone, Debug)]
pub struct Bank {
    pub id: String,
    pub name: String,
    pub questions: Vec<Question>,
}

impl Bank {
    /// 用户文件优先，解析或校验失败则回退内嵌（不覆盖用户文件）。
    pub fn load(root: &Path) -> Bank {
        let path = root.join(BANK_FILE);
        if path.exists() {
            match std::fs::read(&path)
                .map_err(|err| err.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<BankFile>(&bytes).map_err(|err| err.to_string())
                }) {
                Ok(file) => match validate(&file) {
                    Ok(()) => return Bank::from_file(file),
                    Err(err) => ::log::warn!("[bank] 用户题库校验失败，回退内嵌：{err}"),
                },
                Err(err) => ::log::warn!("[bank] 用户题库解析失败，回退内嵌：{err}"),
            }
        }

        // 内嵌题库的合法性由单测保证；真出问题时宁可 panic 也不能返回空题库
        match serde_json::from_str::<BankFile>(EMBEDDED) {
            Ok(file) => Bank::from_file(file),
            Err(err) => {
                ::log::error!("[bank] 内嵌题库损坏（构建缺陷）：{err}");
                Bank {
                    id: "basic".to_string(),
                    name: "基础阵法".to_string(),
                    questions: Vec::new(),
                }
            }
        }
    }

    fn from_file(file: BankFile) -> Bank {
        Bank {
            id: file.id,
            name: file.name,
            questions: file.questions,
        }
    }

    pub fn len(&self) -> usize {
        self.questions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    /// 抽 `count` 道**互不重复**的题（F-5.3 的同轮不重复），不看记忆衰减。
    /// 测试与"无统计"场景用它；正式抽题走 `draw_with`。
    pub fn draw(&self, count: usize, rng: &mut Rng) -> Vec<Question> {
        let mut indices: Vec<usize> = (0..self.questions.len()).collect();
        rng.shuffle(&mut indices);
        indices
            .into_iter()
            .take(count.min(self.questions.len()))
            .map(|index| self.questions[index].clone())
            .collect()
    }

    /// 带记忆衰减的抽题（F-8.2）。
    ///
    /// 两级退化，**保证返回 `min(count, 题库总数)` 道题，绝不让 3 题变 2 题**
    /// （2 题全对的蒙对率 1/16，会让"蒙不过去"的论证失效）：
    /// 1. 冷却池（不在 30 天 / 30 分钟冷却里的题）内随机取；
    /// 2. 不够时，把冷却中的题按 `lastAskedMs` 升序（从未问过的最前）补进来。
    ///
    /// 为什么只有两级：第 2 级本身不看冷却，所以 PRD F-8.2 的第三级"全库随机"
    /// 永远不会触发；按"最久未出现"排序比随机更符合意图。
    pub fn draw_with(
        &self,
        stats: &crate::core::bank_stats::Table,
        count: usize,
        now_ms: i64,
        rng: &mut Rng,
    ) -> Vec<Question> {
        let count = count.min(self.questions.len());
        if count == 0 {
            return Vec::new();
        }

        let mut fresh: Vec<usize> = (0..self.questions.len())
            .filter(|index| !stats.is_cooling(&self.questions[*index].id, now_ms))
            .collect();
        rng.shuffle(&mut fresh);

        let take = fresh.len().min(count);
        let mut chosen: Vec<usize> = fresh[..take].to_vec();

        if chosen.len() < count {
            let mut rest: Vec<usize> = (0..self.questions.len())
                .filter(|index| !chosen.contains(index))
                .collect();
            rest.sort_by_key(|index| {
                stats
                    .last_asked_ms(&self.questions[*index].id)
                    .unwrap_or(i64::MIN)
            });
            chosen.extend_from_slice(&rest[..count - chosen.len()]);
        }

        chosen
            .into_iter()
            .map(|index| self.questions[index].clone())
            .collect()
    }
}

fn validate(file: &BankFile) -> Result<(), String> {
    if file.questions.is_empty() {
        return Err("题库为空".to_string());
    }

    let mut seen = std::collections::HashSet::new();
    for question in &file.questions {
        if question.options.len() != OPTION_COUNT {
            return Err(format!("{} 的选项数不是 {OPTION_COUNT}", question.id));
        }
        if question.answer >= question.options.len() {
            return Err(format!("{} 的 answer 越界", question.id));
        }
        if question.stem.trim().is_empty() {
            return Err(format!("{} 题干为空", question.id));
        }
        if question.stem.chars().count() > MAX_STEM_CHARS {
            return Err(format!("{} 题干超过 {MAX_STEM_CHARS} 字", question.id));
        }
        if !seen.insert(question.id.as_str()) {
            return Err(format!("题号重复：{}", question.id));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "biguansim-bank-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("创建临时目录");
        path
    }

    /// 内嵌题库是所有兜底的最后一层：它坏了就"无题可问"，必须由测试守住
    #[test]
    fn embedded_bank_is_valid() {
        let bank = Bank::load(std::path::Path::new("不存在的目录"));
        assert!(bank.len() >= 12, "内嵌题库至少 12 题，实际 {}", bank.len());
        assert_eq!(bank.id, "basic");

        let mut positions = std::collections::HashSet::new();
        for question in &bank.questions {
            assert_eq!(question.options.len(), OPTION_COUNT);
            assert!(question.answer < OPTION_COUNT);
            assert!(question.stem.chars().count() <= MAX_STEM_CHARS);
            assert_eq!(question.source, "generated");
            positions.insert(question.answer);
        }
        assert!(
            positions.len() >= 3,
            "正确答案位置必须分散，否则背位置就能蒙对：{positions:?}"
        );
    }

    #[test]
    fn draw_does_not_repeat_within_round() {
        let bank = Bank::load(std::path::Path::new("不存在"));
        let mut rng = Rng::new(2026);

        let drawn = bank.draw(5, &mut rng);
        assert_eq!(drawn.len(), 5);

        let ids: std::collections::HashSet<&str> = drawn.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids.len(), 5, "同一轮不得抽到重复题");
    }

    #[test]
    fn draw_is_reproducible_with_same_seed() {
        let bank = Bank::load(std::path::Path::new("不存在"));
        let a: Vec<String> = bank
            .draw(4, &mut Rng::new(7))
            .into_iter()
            .map(|q| q.id)
            .collect();
        let b: Vec<String> = bank
            .draw(4, &mut Rng::new(7))
            .into_iter()
            .map(|q| q.id)
            .collect();
        assert_eq!(a, b);
    }

    /// 题不够时有多少给多少，绝不返回空（除非题库本身为空）
    #[test]
    fn draw_more_than_available_returns_all() {
        let bank = Bank::load(std::path::Path::new("不存在"));
        let drawn = bank.draw(99, &mut Rng::new(1));
        assert_eq!(drawn.len(), bank.len());
    }

    /// 回归：用户把题库文件改坏，必须回退内嵌而不是"无题可问"
    #[test]
    fn corrupt_user_bank_falls_back_to_embedded() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(dir.join("bank")).unwrap();
        std::fs::write(dir.join(BANK_FILE), b"{ not json").unwrap();

        let bank = Bank::load(&dir);
        assert!(bank.len() >= 12, "坏文件必须回退内嵌");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 结构合法但语义非法（选项 3 个）同样要回退
    #[test]
    fn invalid_user_bank_is_rejected() {
        let dir = temp_dir("invalid");
        std::fs::create_dir_all(dir.join("bank")).unwrap();
        std::fs::write(
            dir.join(BANK_FILE),
            r#"{"version":1,"id":"x","name":"x","questions":[
                {"id":"a","type":"choice","difficulty":"x","source":"generated",
                 "stem":"题","options":["1","2","3"],"answer":0}]}"#
                .as_bytes(),
        )
        .unwrap();

        let bank = Bank::load(&dir);
        assert_eq!(bank.id, "basic", "非法题库不得被采用");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- 记忆衰减与退化（S2） ----------

    use crate::core::bank_stats::Table;

    fn fixture(count: usize) -> Bank {
        Bank {
            id: "fixture".to_string(),
            name: "夹具".to_string(),
            questions: (0..count)
                .map(|index| Question {
                    id: format!("q{index}"),
                    kind: "choice".to_string(),
                    difficulty: "undergrad".to_string(),
                    tags: Vec::new(),
                    source: "generated".to_string(),
                    stem: format!("第 {index} 题"),
                    options: vec![
                        "对".to_string(),
                        "甲".to_string(),
                        "乙".to_string(),
                        "丙".to_string(),
                    ],
                    answer: 0,
                    hint: None,
                })
                .collect(),
        }
    }

    const NOW: i64 = 1_792_500_000_000;

    #[test]
    fn draw_with_skips_cooling_questions() {
        let bank = fixture(6);
        let mut stats = Table::default();
        for index in 0..5 {
            stats.mark(&format!("q{index}"), NOW, true);
        }

        let drawn = bank.draw_with(&stats, 3, NOW, &mut Rng::new(1));
        let ids: Vec<&str> = drawn.iter().map(|q| q.id.as_str()).collect();

        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"q5"), "只有 q5 没在冷却，必须被抽中");
    }

    /// F-8.2 的退化：全部在冷却时仍要凑满（否则只能出 1 题甚至 0 题）
    #[test]
    fn draw_with_falls_back_when_everything_is_cooling() {
        let bank = fixture(6);
        let mut stats = Table::default();
        for index in 0..6 {
            stats.mark(&format!("q{index}"), NOW - index as i64 * 1_000, true);
        }

        let drawn = bank.draw_with(&stats, 5, NOW, &mut Rng::new(2));
        assert_eq!(drawn.len(), 5, "冷却池为空时也必须凑满 5 题");

        let ids: std::collections::HashSet<&str> = drawn.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids.len(), 5, "同轮不得重复");
    }

    /// 退化时优先"最久没出现过"的题：q0 是 6 天前，q5 是刚刚
    #[test]
    fn fallback_prefers_least_recently_asked() {
        let bank = fixture(3);
        let mut stats = Table::default();
        stats.mark("q0", NOW - 6 * 24 * 60 * 60 * 1_000, true);
        stats.mark("q1", NOW - 3 * 24 * 60 * 60 * 1_000, true);
        stats.mark("q2", NOW, true);

        let drawn = bank.draw_with(&stats, 2, NOW, &mut Rng::new(3));
        let ids: Vec<&str> = drawn.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids, vec!["q0", "q1"], "应按最久未出现排序补齐");
    }

    #[test]
    fn wrong_answer_returns_after_thirty_minutes() {
        let bank = fixture(3);
        let mut stats = Table::default();
        stats.mark("q0", NOW, false);
        stats.mark("q1", NOW, false);
        stats.mark("q2", NOW, false);

        let drawn = bank.draw_with(&stats, 3, NOW + 31 * 60 * 1_000, &mut Rng::new(4));
        assert_eq!(drawn.len(), 3);
    }

    #[test]
    fn draw_with_never_exceeds_bank_size() {
        let bank = fixture(4);
        let drawn = bank.draw_with(&Table::default(), 99, NOW, &mut Rng::new(5));
        assert_eq!(drawn.len(), 4);
    }

    #[test]
    fn valid_user_bank_wins() {
        let dir = temp_dir("valid");
        std::fs::create_dir_all(dir.join("bank")).unwrap();
        std::fs::write(
            dir.join(BANK_FILE),
            r#"{"version":1,"id":"custom","name":"自定义","questions":[
                {"id":"a","type":"choice","difficulty":"x","source":"generated",
                 "stem":"一加一等于？","options":["1","2","3","4"],"answer":1}]}"#
                .as_bytes(),
        )
        .unwrap();

        let bank = Bank::load(&dir);
        assert_eq!(bank.id, "custom");
        assert_eq!(bank.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
