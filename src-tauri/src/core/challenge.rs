//! 问阵（Challenge）：题目打乱、逐题判定（PRD F-5.1 ~ F-5.6）。
//!
//! 三条铁律（TDD §5.1）：
//! 1. **答案只留在 Rust**：`Prepared` 有 `answer_display`，对外只有 `QuestionView`；
//! 2. **前端只回传展示索引**（0–3），映射由本模块持有，越界即 `Invalid`；
//! 3. 错 1 题即失败，后续题不再作答（`submit` 返回 `Wrong` 后调用方直接回冷却）。

use serde::Serialize;

use crate::core::bank::Question;
use crate::core::rng::Rng;

/// 一道已经打乱选项、可以直接下发的题
#[derive(Clone, Debug)]
pub struct Prepared {
    id: String,
    stem: String,
    options: Vec<String>,
    /// 正确项在**展示顺序**中的位置
    answer_display: usize,
}

/// 下发给前端的题面：**结构里没有答案字段**，序列化后也不可能带出去
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionView {
    /// 第几题（1 起算）
    pub index: usize,
    pub total: usize,
    pub stem: String,
    pub options: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judgement {
    /// 索引越界或题号对不上：丢弃本次点击，不推进
    Invalid,
    /// 答错：整轮失败，调用方转 Cool（`attempts += 1`）
    Wrong,
    /// 答对了，后面还有题
    Next,
    /// 答对且是最后一题（整轮全对）
    AllCorrect,
}

#[derive(Clone, Debug)]
pub struct Round {
    attempt: u32,
    questions: Vec<Prepared>,
    index: usize,
}

impl Round {
    /// 打乱每题选项。`questions` 由 `bank::draw` 给出（同轮不重复）。
    pub fn prepare(questions: Vec<Question>, attempt: u32, rng: &mut Rng) -> Round {
        let prepared = questions
            .into_iter()
            .filter(|question| question.options.len() >= 2)
            .map(|question| {
                let mut pairs: Vec<(String, bool)> = question
                    .options
                    .iter()
                    .enumerate()
                    .map(|(position, text)| (text.clone(), position == question.answer))
                    .collect();
                rng.shuffle(&mut pairs);

                let answer_display = pairs.iter().position(|(_, correct)| *correct).unwrap_or(0);
                let options: Vec<String> = pairs.into_iter().map(|(text, _)| text).collect();

                Prepared {
                    id: question.id,
                    stem: question.stem,
                    options,
                    answer_display,
                }
            })
            .collect();

        Round {
            attempt,
            questions: prepared,
            index: 0,
        }
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    pub fn total(&self) -> usize {
        self.questions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    /// 当前题号（1 起算）；已答完返回 0
    pub fn position(&self) -> usize {
        self.index + 1
    }

    pub fn current_id(&self) -> Option<&str> {
        self.questions.get(self.index).map(|q| q.id.as_str())
    }

    pub fn view(&self) -> Option<QuestionView> {
        let question = self.questions.get(self.index)?;
        Some(QuestionView {
            index: self.index + 1,
            total: self.questions.len(),
            stem: question.stem.clone(),
            options: question.options.clone(),
        })
    }

    /// 提交一次选择。`choice` 是**展示索引**。
    pub fn submit(&mut self, choice: usize) -> Judgement {
        let Some(question) = self.questions.get(self.index) else {
            return Judgement::Invalid;
        };

        if choice >= question.options.len() {
            return Judgement::Invalid;
        }

        if choice != question.answer_display {
            return Judgement::Wrong;
        }

        self.index += 1;
        if self.index >= self.questions.len() {
            Judgement::AllCorrect
        } else {
            Judgement::Next
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bank::Question;

    fn question(id: &str, answer: usize) -> Question {
        Question {
            id: id.to_string(),
            kind: "choice".to_string(),
            difficulty: "undergrad".to_string(),
            tags: Vec::new(),
            source: "generated".to_string(),
            stem: format!("问题 {id}"),
            options: (0..4).map(|i| format!("{id}-opt{i}")).collect(),
            answer,
            hint: None,
        }
    }

    fn round(count: usize, seed: u64) -> Round {
        let questions: Vec<Question> = (0..count)
            .map(|i| question(&format!("q{i}"), i % 4))
            .collect();
        Round::prepare(questions, 1, &mut Rng::new(seed))
    }

    /// 打乱必须保持"正确答案仍在原位置语义上"——否则会把对题判成错题
    #[test]
    fn shuffle_keeps_answer_mapping() {
        for seed in 0..32u64 {
            let mut rng = Rng::new(seed);
            let source = question("q", 2);
            let expected = source.options[2].clone();
            let prepared = Round::prepare(vec![source], 1, &mut rng);

            let view = prepared.view().expect("有题");
            assert_eq!(view.options[prepared.questions[0].answer_display], expected);
        }
    }

    #[test]
    fn shuffle_actually_changes_something() {
        let mut changed = false;
        for seed in 0..16u64 {
            let prepared = round(4, seed);
            for (position, question) in prepared.questions.iter().enumerate() {
                if question.options
                    != (0..4)
                        .map(|i| format!("q{position}-opt{i}"))
                        .collect::<Vec<_>>()
                {
                    changed = true;
                }
            }
        }
        assert!(changed, "32 次里一次都没打乱，说明 shuffle 没生效");
    }

    #[test]
    fn correct_answers_advance_to_all_correct() {
        let mut r = round(3, 7);
        for _ in 0..2 {
            let answer = r.questions[r.index].answer_display;
            assert_eq!(r.submit(answer), Judgement::Next);
        }
        let answer = r.questions[r.index].answer_display;
        assert_eq!(r.submit(answer), Judgement::AllCorrect);
    }

    #[test]
    fn wrong_answer_does_not_advance() {
        let mut r = round(3, 11);
        let answer = r.questions[0].answer_display;
        let wrong = (answer + 1) % 4;

        assert_eq!(r.submit(wrong), Judgement::Wrong);
        assert_eq!(r.position(), 1, "答错不得推进题号");
        assert_eq!(r.current_id(), Some("q0"));
    }

    #[test]
    fn out_of_range_choice_is_invalid() {
        let mut r = round(2, 3);
        assert_eq!(r.submit(4), Judgement::Invalid);
        assert_eq!(r.submit(99), Judgement::Invalid);
        assert_eq!(r.position(), 1, "非法点击不得推进");
    }

    #[test]
    fn view_reports_position_and_total() {
        let mut r = round(3, 5);
        let first = r.view().expect("有题");
        assert_eq!(first.index, 1);
        assert_eq!(first.total, 3);
        assert_eq!(first.options.len(), 4);

        let answer = r.questions[0].answer_display;
        r.submit(answer);
        assert_eq!(r.view().expect("有题").index, 2);
    }

    /// A-25a：任何下发结构都不得含 `answer` 字段
    #[test]
    fn view_never_contains_answer() {
        let r = round(2, 13);
        let json = serde_json::to_string(&r.view().unwrap()).unwrap();
        assert!(!json.contains("answer"), "题面里出现了 answer：{json}");
    }

    #[test]
    fn empty_round_has_no_view() {
        let round = Round::prepare(Vec::new(), 1, &mut Rng::new(1));
        assert!(round.is_empty());
        assert!(round.view().is_none());
        assert_eq!(round.current_id(), None);
    }
}
