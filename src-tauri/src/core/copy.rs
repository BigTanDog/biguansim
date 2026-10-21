//! 抄写判定（PRD F-5.8 / A-23）。
//!
//! 逐字比对，**不做任何归一化**：全角逗号与半角逗号算不同字符。
//! 理由写在 F-5.8：归一化会让"抄错"变成"抄对"，而抄写是这段流程里唯一的体力成本；
//! 反过来，语料本身已限定为纯中文与中文标点（有单测守住），所以也不会出现
//! "全角半角没法打出来"的无效挫败。

use serde::Serialize;

/// 一次抄写任务：原文 + 字数（字数用于界面提示"共 N 字"）
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTask {
    pub text: String,
    pub chars: u32,
}

impl CopyTask {
    pub fn new(text: String) -> Self {
        let chars = text.chars().count() as u32;
        Self { text, chars }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyVerdict {
    Ok,
    /// 第一个不一致的位置（字符序号，从 0 起）。少字时等于已抄长度，多字时等于原文字数。
    Mismatch {
        index: usize,
    },
}

/// 逐字校验。`typed` 是用户输入的全部内容，原样比对。
pub fn check(target: &str, typed: &str) -> CopyVerdict {
    let target: Vec<char> = target.chars().collect();
    let typed: Vec<char> = typed.chars().collect();

    for (index, expected) in target.iter().enumerate() {
        match typed.get(index) {
            Some(actual) if actual == expected => {}
            _ => return CopyVerdict::Mismatch { index },
        }
    }

    if typed.len() > target.len() {
        return CopyVerdict::Mismatch {
            index: target.len(),
        };
    }

    CopyVerdict::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "道友且坐，莫问几时。";

    #[test]
    fn identical_text_passes() {
        assert_eq!(check(TARGET, TARGET), CopyVerdict::Ok);
    }

    #[test]
    fn one_wrong_char_reports_index() {
        let typed = "道友且座，莫问几时。";
        assert_eq!(check(TARGET, typed), CopyVerdict::Mismatch { index: 3 });
    }

    /// 少 1 字：报在缺失处，前端据此标红
    #[test]
    fn missing_char_is_mismatch() {
        let typed = "道友且坐莫问几时。";
        assert_eq!(check(TARGET, typed), CopyVerdict::Mismatch { index: 4 });
    }

    /// 多 1 字：多出来的位置就是原文长度
    #[test]
    fn extra_char_is_mismatch() {
        let typed = "道友且坐，莫问几时。。";
        assert_eq!(
            check(TARGET, typed),
            CopyVerdict::Mismatch {
                index: TARGET.chars().count()
            }
        );
    }

    /// 全角/半角不归一化：半角逗号必须判错
    #[test]
    fn half_width_punctuation_is_rejected() {
        let typed = "道友且坐,莫问几时。";
        assert!(matches!(check(TARGET, typed), CopyVerdict::Mismatch { .. }));
    }

    #[test]
    fn empty_input_fails_at_first_char() {
        assert_eq!(check(TARGET, ""), CopyVerdict::Mismatch { index: 0 });
    }

    /// 按字符（不是字节）比对：中文与 emoji 都不能把索引算歪
    #[test]
    fn compares_by_chars_not_bytes() {
        assert_eq!(check("一二三", "一二三"), CopyVerdict::Ok);
        assert_eq!(
            check("一二三", "一二四"),
            CopyVerdict::Mismatch { index: 2 }
        );
    }
}
