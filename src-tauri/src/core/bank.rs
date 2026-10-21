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

    /// 抽 `count` 道**互不重复**的题（F-5.3 的同轮不重复）。
    /// 题少时有多少给多少；记忆衰减与三级退化在 M5 接进来，接口不变。
    pub fn draw(&self, count: usize, rng: &mut Rng) -> Vec<Question> {
        let mut indices: Vec<usize> = (0..self.questions.len()).collect();
        rng.shuffle(&mut indices);
        indices
            .into_iter()
            .take(count.min(self.questions.len()))
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
