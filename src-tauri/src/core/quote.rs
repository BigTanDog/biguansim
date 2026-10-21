//! 长老语料：阶段划分、按场景取句、变量渲染、抄写段抽取（PRD §10 / F-4.4 / F-5.7 / F-5.8）。
//!
//! 与题库同样的兜底策略：用户文件 `%APPDATA%\闭关模拟器\quotes.json` 解析失败
//! 或结构不全 → 回退内嵌（不覆盖用户文件）。语料为空会让对话框变成空白，
//! 那是最糟的观感事故，因此这里宁可整份回退。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::rng::Rng;

pub const QUOTES_FILE: &str = "quotes.json";
/// 抄写段与目标字数的允许偏差（M4 计划 D-29：取整段原文，不截断）
pub const COPY_LEN_TOLERANCE: u32 = 20;
/// 语料全丢时的兜底句（真实句子，不是占位符）
const FALLBACK: &str = "道友，且坐。";

const EMBEDDED: &str = include_str!("../../resources/quotes.json");

/// 语料阶段（T-2）：0–30 分 / 30–90 分 / 90 分以上
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Early,
    Mid,
    Late,
}

impl Stage {
    pub fn of_elapsed_ms(elapsed_ms: u64) -> Stage {
        let minutes = elapsed_ms / 60_000;
        if minutes < 30 {
            Stage::Early
        } else if minutes < 90 {
            Stage::Mid
        } else {
            Stage::Late
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Early => "early",
            Stage::Mid => "mid",
            Stage::Late => "late",
        }
    }
}

/// 语料场景：叙述 / 答错训话 / 冷却轮换
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteKind {
    Idle,
    Wrong,
    Cooldown,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct StageQuotes {
    #[serde(default)]
    idle: Vec<String>,
    #[serde(default)]
    wrong: Vec<String>,
    #[serde(default)]
    cooldown: Vec<String>,
}

impl StageQuotes {
    fn list(&self, kind: QuoteKind) -> &[String] {
        match kind {
            QuoteKind::Idle => &self.idle,
            QuoteKind::Wrong => &self.wrong,
            QuoteKind::Cooldown => &self.cooldown,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct QuoteFile {
    early: StageQuotes,
    mid: StageQuotes,
    late: StageQuotes,
    #[serde(default)]
    copy: Vec<String>,
}

impl QuoteFile {
    fn stage(&self, stage: Stage) -> &StageQuotes {
        match stage {
            Stage::Early => &self.early,
            Stage::Mid => &self.mid,
            Stage::Late => &self.late,
        }
    }

    fn is_complete(&self) -> bool {
        let kinds = [QuoteKind::Idle, QuoteKind::Wrong, QuoteKind::Cooldown];
        let stages = [Stage::Early, Stage::Mid, Stage::Late];
        stages.iter().all(|stage| {
            kinds
                .iter()
                .all(|kind| !self.stage(*stage).list(*kind).is_empty())
        }) && !self.copy.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct QuoteBook {
    file: QuoteFile,
}

impl QuoteBook {
    pub fn load(root: &Path) -> QuoteBook {
        let path = root.join(QUOTES_FILE);
        if path.exists() {
            match std::fs::read(&path)
                .map_err(|err| err.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<QuoteFile>(&bytes).map_err(|err| err.to_string())
                }) {
                Ok(file) if file.is_complete() => return QuoteBook { file },
                Ok(_) => ::log::warn!("[quote] 用户语料结构不全，回退内嵌"),
                Err(err) => ::log::warn!("[quote] 用户语料解析失败，回退内嵌：{err}"),
            }
        }

        match serde_json::from_str::<QuoteFile>(EMBEDDED) {
            Ok(file) => QuoteBook { file },
            Err(err) => {
                // 内嵌都坏了属于构建缺陷：记 error，用兜底句继续跑（不能因此锁死流程）
                ::log::error!("[quote] 内嵌语料损坏（构建缺陷）：{err}");
                QuoteBook {
                    file: QuoteFile {
                        early: StageQuotes::default(),
                        mid: StageQuotes::default(),
                        late: StageQuotes::default(),
                        copy: Vec::new(),
                    },
                }
            }
        }
    }

    /// 按阶段与场景随机取一句；语料缺失时返回兜底句。
    pub fn pick(&self, stage: Stage, kind: QuoteKind, rng: &mut Rng) -> String {
        let list = self.file.stage(stage).list(kind);
        if list.is_empty() {
            return FALLBACK.to_string();
        }
        list[rng.below(list.len() as u32) as usize].clone()
    }

    /// 取一段最接近 `target_chars` 的抄写原文（±20 字内随机，取不到就取最接近的）。
    pub fn copy_line(&self, target_chars: u32, rng: &mut Rng) -> Option<String> {
        let target = target_chars as i64;
        let mut candidates: Vec<&String> = self
            .file
            .copy
            .iter()
            .filter(|text| {
                let len = text.chars().count() as i64;
                (len - target).abs() <= i64::from(COPY_LEN_TOLERANCE)
            })
            .collect();

        if candidates.is_empty() {
            let best = self.file.copy.iter().min_by_key(|text| {
                let len = text.chars().count() as i64;
                (len - target).abs()
            })?;
            return Some(best.clone());
        }

        let index = rng.below(candidates.len() as u32) as usize;
        Some(candidates.swap_remove(index).clone())
    }

    pub fn copy_count(&self) -> usize {
        self.file.copy.len()
    }
}

/// 变量替换（T-4）：`{attempts}` / `{elapsed}` / `{faults}` / `{days}` / `{target}`。
pub fn render(line: &str, vars: &[(&str, &str)]) -> String {
    let mut out = line.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> QuoteBook {
        QuoteBook::load(Path::new("不存在的目录"))
    }

    /// 阶段边界必须与 PRD T-2 完全一致
    #[test]
    fn stage_boundaries() {
        assert_eq!(Stage::of_elapsed_ms(0), Stage::Early);
        assert_eq!(Stage::of_elapsed_ms(29 * 60_000 + 59_999), Stage::Early);
        assert_eq!(Stage::of_elapsed_ms(30 * 60_000), Stage::Mid);
        assert_eq!(Stage::of_elapsed_ms(89 * 60_000 + 59_999), Stage::Mid);
        assert_eq!(Stage::of_elapsed_ms(90 * 60_000), Stage::Late);
    }

    #[test]
    fn embedded_quotes_cover_all_cells() {
        let book = book();
        for stage in [Stage::Early, Stage::Mid, Stage::Late] {
            for kind in [QuoteKind::Idle, QuoteKind::Wrong, QuoteKind::Cooldown] {
                let line = book.pick(stage, kind, &mut Rng::new(1));
                assert!(!line.is_empty(), "{} {:?} 取到空句", stage.as_str(), kind);
            }
        }
        assert!(book.copy_count() >= 4, "抄写段至少 4 段");
    }

    /// F-5.8：抄写文本只含中文字符与中文标点，不得含英文、数字、半角标点。
    /// 用全角/半角造成的无效挫败是最伤体验的一种失败。
    #[test]
    fn copy_texts_are_chinese_only() {
        // 破折号「——」也是中文标点，抄写里允许
        const PUNCT: &str = "。，、；：？！“”‘’（）《》「」—";
        let book = book();
        for text in &book.file.copy {
            let bad: Vec<char> = text
                .chars()
                .filter(|c| !('\u{4e00}'..='\u{9fff}').contains(c) && !PUNCT.contains(*c))
                .collect();
            assert!(bad.is_empty(), "出现非中文/非中文标点字符：{bad:?}");
        }
    }

    /// D-29：抄写段落长度落在目标附近，不能出现"100 字的目标配 260 字的原文"
    #[test]
    fn copy_lengths_match_targets() {
        let book = book();
        let short = book.copy_line(100, &mut Rng::new(3)).expect("短段落");
        let long = book.copy_line(200, &mut Rng::new(3)).expect("长段落");

        let short_len = short.chars().count() as i64;
        let long_len = long.chars().count() as i64;
        assert!(
            (short_len - 100).abs() <= i64::from(COPY_LEN_TOLERANCE),
            "{short_len}"
        );
        assert!(
            (long_len - 200).abs() <= i64::from(COPY_LEN_TOLERANCE),
            "{long_len}"
        );
    }

    /// 极短题库也要给出答案：没有落在容差内的段落时取最接近的一段
    #[test]
    fn copy_line_falls_back_to_closest() {
        let book = book();
        let text = book.copy_line(1_000, &mut Rng::new(5)).expect("必须有段落");
        assert!(!text.is_empty());
    }

    #[test]
    fn render_replaces_known_vars() {
        let out = render(
            "第 {attempts} 次，已坐 {elapsed}。",
            &[("attempts", "3"), ("elapsed", "32 分")],
        );
        assert_eq!(out, "第 3 次，已坐 32 分。");
    }

    #[test]
    fn corrupt_user_file_falls_back() {
        let dir = std::env::temp_dir().join(format!("biguansim-quote-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(QUOTES_FILE), b"{ broken").unwrap();

        let book = QuoteBook::load(&dir);
        assert!(book.copy_count() >= 4, "坏文件必须回退内嵌");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
