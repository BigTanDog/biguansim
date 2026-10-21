use super::*;
use crate::core::bank::Question;
use crate::core::copy::CopyTask;
use crate::core::rng::Rng;

const NOW_MS: i64 = 1_760_875_920_000;
/// 测试里统一用 1 小时的闭关时长，方便构造"剩余不足"的边界
const HOUR_MS: u64 = 3_600_000;

fn machine() -> SessionMachine {
    SessionMachine::new()
}

fn start(machine: &mut SessionMachine, duration_ms: u64) {
    let outcome = machine
        .transition(
            SessionEvent::Start {
                duration_ms,
                now_ms: NOW_MS,
                now_mono_ms: 1_000,
                goal: Some("背完 List 5–7".to_string()),
                bank: "basic".to_string(),
                cooldown_base: 5,
                question_base: 3,
                // 单测按正式版口径（不缩放冷却），否则断言的 5/10/15 分钟都会变
                cost_scale: 1,
                speaker: "长老".to_string(),
            },
            0,
        )
        .expect("入关应当成功");
    assert!(matches!(outcome, Outcome::Arming));
}

fn arm_and_lock(machine: &mut SessionMachine, duration_ms: u64) {
    start(machine, duration_ms);
    let outcome = machine
        .transition(SessionEvent::ArmFinished, 0)
        .expect("起势结束");
    assert!(matches!(outcome, Outcome::Locked));
}

/// 造一轮题：每题的正确项文本都是「对」，因此可以从 view 的展示顺序里找到它
fn round(count: usize) -> Round {
    let questions: Vec<Question> = (0..count)
        .map(|index| Question {
            id: format!("q{index}"),
            kind: "choice".to_string(),
            difficulty: "undergrad".to_string(),
            tags: Vec::new(),
            source: "generated".to_string(),
            stem: format!("第 {index} 题"),
            options: vec![
                "对".to_string(),
                "错一".to_string(),
                "错二".to_string(),
                "错三".to_string(),
            ],
            answer: 0,
            hint: None,
        })
        .collect();
    Round::prepare(questions, 1, &mut Rng::new(2026))
}

fn correct_index(round: &Round) -> usize {
    round
        .view()
        .expect("有题")
        .options
        .iter()
        .position(|text| text == "对")
        .expect("正确项一定存在")
}

fn wrong_index(round: &Round) -> usize {
    (correct_index(round) + 1) % 4
}

fn copy_task() -> CopyTask {
    CopyTask::new("道友且坐，莫问几时。".to_string())
}

/// 把机器推到第 `attempts` 次尝试的 Challenge 状态（绕过 5/10/15 分钟的真实等待）
fn lock_with_attempts(machine: &mut SessionMachine, attempts: u32) {
    arm_and_lock(machine, HOUR_MS);
    if let Some(session) = machine.session.as_mut() {
        session.attempts = attempts;
    }
    machine.state = SessionState::Challenge;
    if let Some(session) = machine.session.as_mut() {
        session.round = Some(round(3));
    }
}

#[test]
fn start_from_idle_enters_arming() {
    let mut m = machine();
    start(&mut m, 60_000);

    assert_eq!(m.state(), SessionState::Arming);
    assert_eq!(m.session().unwrap().id, "s_1760875920000");
    assert_eq!(m.session().unwrap().elapsed_ms, 0);
}

#[test]
fn arming_finished_enters_locked() {
    let mut m = machine();
    start(&mut m, 60_000);

    let outcome = m.transition(SessionEvent::ArmFinished, 0).expect("起势");
    assert!(matches!(outcome, Outcome::Locked));
    assert_eq!(m.state(), SessionState::Locked);
}

#[test]
fn cancel_during_arming_returns_to_idle() {
    let mut m = machine();
    start(&mut m, 60_000);

    let outcome = m.transition(SessionEvent::Cancel, 0).expect("取消");
    assert!(matches!(outcome, Outcome::Idle));
    assert_eq!(m.state(), SessionState::Idle);
    assert!(m.session().is_none(), "取消后不得留下会话");
}

/// 回归：Arming 期间没有计时器，任何 tick 都不得触发结算
#[test]
fn tick_during_arming_does_not_settle() {
    let mut m = machine();
    start(&mut m, 60_000);

    let outcome = m
        .transition(SessionEvent::Tick, 9_999_999)
        .expect("Arming 下的 tick 不是非法事件");

    assert!(matches!(outcome, Outcome::Unchanged));
    assert_eq!(m.state(), SessionState::Arming);
}

#[test]
fn start_while_arming_is_illegal() {
    let mut m = machine();
    start(&mut m, 60_000);

    let err = m
        .transition(
            SessionEvent::Start {
                duration_ms: 60_000,
                now_ms: NOW_MS,
                now_mono_ms: 2_000,
                goal: None,
                bank: "basic".to_string(),
                cooldown_base: 5,
                question_base: 3,
                cost_scale: 1,
                speaker: "长老".to_string(),
            },
            0,
        )
        .unwrap_err();

    assert_eq!(err, SessionError::StateIllegal);
}

#[test]
fn start_while_locked_is_illegal() {
    let mut m = machine();
    arm_and_lock(&mut m, 60_000);

    let err = m
        .transition(
            SessionEvent::Start {
                duration_ms: 60_000,
                now_ms: NOW_MS,
                now_mono_ms: 2_000,
                goal: None,
                bank: "basic".to_string(),
                cooldown_base: 5,
                question_base: 3,
                cost_scale: 1,
                speaker: "长老".to_string(),
            },
            0,
        )
        .unwrap_err();

    assert_eq!(err, SessionError::StateIllegal);
}

#[test]
fn start_while_cool_is_illegal() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestUnlock, 0).expect("解锁");

    let err = m
        .transition(
            SessionEvent::Start {
                duration_ms: 60_000,
                now_ms: NOW_MS,
                now_mono_ms: 2_000,
                goal: None,
                bank: "basic".to_string(),
                cooldown_base: 5,
                question_base: 3,
                cost_scale: 1,
                speaker: "长老".to_string(),
            },
            0,
        )
        .unwrap_err();

    assert_eq!(err, SessionError::StateIllegal);
}

#[test]
fn finish_during_arming_is_illegal() {
    let mut m = machine();
    start(&mut m, 60_000);

    let err = m
        .transition(
            SessionEvent::Finish {
                result: SessionResult::Emergency,
            },
            0,
        )
        .unwrap_err();

    assert_eq!(err, SessionError::StateIllegal);
}

#[test]
fn cancel_outside_arming_is_illegal() {
    let mut m = machine();
    arm_and_lock(&mut m, 60_000);

    let err = m.transition(SessionEvent::Cancel, 0).unwrap_err();
    assert_eq!(err, SessionError::StateIllegal);
}

/// 回归：Idle 下 remaining 天然为 0，到点判定若不带状态前置会误结算
#[test]
fn tick_in_idle_does_not_settle() {
    let mut m = machine();

    let outcome = m
        .transition(SessionEvent::Tick, 0)
        .expect("Idle 下的 tick 不是非法事件");

    assert!(matches!(outcome, Outcome::Unchanged));
    assert_eq!(m.state(), SessionState::Idle);
}

#[test]
fn tick_before_deadline_keeps_locked() {
    let mut m = machine();
    arm_and_lock(&mut m, 60_000);

    let outcome = m.transition(SessionEvent::Tick, 59_999).expect("tick");

    assert!(matches!(outcome, Outcome::Unchanged));
    assert_eq!(m.state(), SessionState::Locked);
    assert_eq!(m.session().unwrap().remaining_ms(), 1);
}

#[test]
fn deadline_settles_as_completed() {
    let mut m = machine();
    arm_and_lock(&mut m, 60_000);

    let outcome = m.transition(SessionEvent::Tick, 60_000).expect("tick");

    match outcome {
        Outcome::Settled { result, session } => {
            assert_eq!(result, SessionResult::Completed);
            assert_eq!(session.elapsed_ms, 60_000);
        }
        _ => panic!("到点必须结算"),
    }
    assert_eq!(m.state(), SessionState::Settled);
    assert!(m.session().is_none());
}

/// 到点优先于"主动结束"：Finish 在到点后也必须是 completed
#[test]
fn deadline_wins_over_explicit_finish() {
    let mut m = machine();
    arm_and_lock(&mut m, 60_000);

    let outcome = m
        .transition(
            SessionEvent::Finish {
                result: SessionResult::Accident,
            },
            90_000,
        )
        .expect("finish");

    match outcome {
        Outcome::Settled { result, .. } => assert_eq!(result, SessionResult::Completed),
        _ => panic!("必须结算"),
    }
}

#[test]
fn finish_without_session_is_illegal() {
    let mut m = machine();

    let err = m
        .transition(
            SessionEvent::Finish {
                result: SessionResult::Completed,
            },
            1_000,
        )
        .unwrap_err();

    assert_eq!(err, SessionError::StateIllegal);
}

#[test]
fn can_start_again_after_settle() {
    let mut m = machine();
    arm_and_lock(&mut m, 1_000);
    let _ = m.transition(SessionEvent::Tick, 1_000).expect("tick");
    assert_eq!(m.state(), SessionState::Settled);

    start(&mut m, 2_000);
    assert_eq!(m.state(), SessionState::Arming);
}

// ---------- 冷却 / 问阵 / 抄写 / 急事 ----------

/// A-4：第一次点「解开阵法」= 5 分钟，attempts 记为 1
#[test]
fn request_unlock_enters_cool_with_five_minutes() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);

    let outcome = m
        .transition(SessionEvent::RequestUnlock, 600_000)
        .expect("解锁");
    match outcome {
        Outcome::Cool {
            attempt,
            cooldown_ms,
        } => {
            assert_eq!(attempt, 1);
            assert_eq!(cooldown_ms, 300_000);
        }
        other => panic!("应进入冷却，实际 {other:?}"),
    }

    assert_eq!(m.state(), SessionState::Cool);
    let session = m.session().unwrap();
    assert_eq!(session.attempts, 1);
    assert_eq!(session.cool_end_elapsed_ms, Some(900_000));
}

/// F-4.5 / A-22：剩余不足一次冷却 → 不动，只有提示
#[test]
fn unlock_refused_when_remaining_shorter_than_cooldown() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);

    let elapsed = HOUR_MS - 299_999; // 剩余 299999 ms < 5 分钟
    let outcome = m
        .transition(SessionEvent::RequestUnlock, elapsed)
        .expect("合法请求");

    match outcome {
        Outcome::UnlockRefused {
            need_ms,
            remaining_ms,
        } => {
            assert_eq!(need_ms, 300_000);
            assert_eq!(remaining_ms, 299_999);
        }
        other => panic!("应被拒绝，实际 {other:?}"),
    }
    assert_eq!(m.state(), SessionState::Locked, "被拒绝后仍留在 Locked");
    assert_eq!(m.session().unwrap().attempts, 0, "被拒绝不得消耗尝试次数");
}

/// 边界：剩余恰好等于一次冷却 → 允许（否则"刚好够"会变成永远进不去）
#[test]
fn unlock_allowed_when_remaining_equals_cooldown() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);

    let elapsed = HOUR_MS - 300_000;
    let outcome = m
        .transition(SessionEvent::RequestUnlock, elapsed)
        .expect("边界应允许");
    assert!(matches!(outcome, Outcome::Cool { .. }));
}

#[test]
fn cooldown_elapsed_only_at_end() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestUnlock, 600_000)
        .expect("解锁");

    let early = m.transition(SessionEvent::Tick, 899_999).expect("tick");
    assert!(matches!(early, Outcome::Unchanged));

    let done = m.transition(SessionEvent::Tick, 900_000).expect("tick");
    assert!(matches!(done, Outcome::CooldownElapsed));
    assert_eq!(m.state(), SessionState::Cool, "到点只报告，不自己转移");
}

/// 回归：冷却没到点就发 CoolFinished = 跳过 5 分钟，必须拒绝
#[test]
fn cool_finished_before_end_is_illegal() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestUnlock, 600_000)
        .expect("解锁");

    let err = m
        .transition(SessionEvent::CoolFinished { round: round(3) }, 899_999)
        .unwrap_err();

    assert_eq!(err, SessionError::StateIllegal);
    assert_eq!(m.state(), SessionState::Cool);
}

#[test]
fn cool_finished_enters_challenge() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestUnlock, 600_000)
        .expect("解锁");

    let outcome = m
        .transition(SessionEvent::CoolFinished { round: round(3) }, 900_000)
        .expect("进问阵");

    assert!(matches!(outcome, Outcome::Challenge));
    assert_eq!(m.state(), SessionState::Challenge);
    assert_eq!(m.session().unwrap().round.as_ref().unwrap().total(), 3);
}

/// 空题绝不进入 Challenge（PRD §12：不能出现"无题可问"的闭关）
#[test]
fn empty_round_is_rejected() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestUnlock, 600_000)
        .expect("解锁");

    let empty = Round::prepare(Vec::new(), 1, &mut Rng::new(1));
    let err = m
        .transition(SessionEvent::CoolFinished { round: empty }, 900_000)
        .unwrap_err();

    assert_eq!(err, SessionError::EmptyRound);
    assert_eq!(m.state(), SessionState::Cool);
}

/// A-6：第 1 次全对（n < 3）直接出关，结果 early
#[test]
fn all_correct_first_attempt_settles_early() {
    let mut m = machine();
    lock_with_attempts(&mut m, 1);

    for _ in 0..2 {
        let index = correct_index(m.session().unwrap().round.as_ref().unwrap());
        let outcome = m
            .transition(
                SessionEvent::SubmitChoice {
                    choice: index,
                    copy: None,
                },
                700_000,
            )
            .expect("答对");
        assert!(matches!(outcome, Outcome::ChallengeAdvanced));
    }

    let index = correct_index(m.session().unwrap().round.as_ref().unwrap());
    let outcome = m
        .transition(
            SessionEvent::SubmitChoice {
                choice: index,
                copy: None,
            },
            800_000,
        )
        .expect("最后一题");

    match outcome {
        Outcome::Settled { result, .. } => assert_eq!(result, SessionResult::Early),
        other => panic!("应结算为 early，实际 {other:?}"),
    }
}

/// A-5：答错 → 回冷却，attempts 递增，冷却变 10 分钟
#[test]
fn wrong_answer_returns_to_cool_with_higher_cost() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestUnlock, 600_000)
        .expect("解锁");
    m.transition(SessionEvent::CoolFinished { round: round(3) }, 900_000)
        .expect("进问阵");

    let wrong = wrong_index(m.session().unwrap().round.as_ref().unwrap());
    let outcome = m
        .transition(
            SessionEvent::SubmitChoice {
                choice: wrong,
                copy: None,
            },
            1_000_000,
        )
        .expect("答错是合法事件");

    match outcome {
        Outcome::Cool {
            attempt,
            cooldown_ms,
        } => {
            assert_eq!(attempt, 2);
            assert_eq!(cooldown_ms, 600_000, "第二次冷却 10 分钟");
        }
        other => panic!("应回冷却，实际 {other:?}"),
    }
    assert_eq!(m.state(), SessionState::Cool);
    assert!(m.session().unwrap().round.is_none(), "答错后题目必须清掉");
}

/// A-5 / A-23：第 3 次全对 → 进抄写（100 字）
#[test]
fn third_attempt_all_correct_enters_copying() {
    let mut m = machine();
    lock_with_attempts(&mut m, 3);
    assert_eq!(m.copy_chars(), 100);

    for _ in 0..2 {
        let index = correct_index(m.session().unwrap().round.as_ref().unwrap());
        m.transition(
            SessionEvent::SubmitChoice {
                choice: index,
                copy: Some(copy_task()),
            },
            700_000,
        )
        .expect("答对");
    }

    let index = correct_index(m.session().unwrap().round.as_ref().unwrap());
    let outcome = m
        .transition(
            SessionEvent::SubmitChoice {
                choice: index,
                copy: Some(copy_task()),
            },
            800_000,
        )
        .expect("全对");

    match outcome {
        Outcome::Copying { target_chars } => assert_eq!(target_chars, 100),
        other => panic!("应进抄写，实际 {other:?}"),
    }
    assert_eq!(m.state(), SessionState::Copying);
    assert!(m.session().unwrap().copy.is_some());
}

/// 需要抄写却没有原文 = 构建缺陷，必须报错而不是悄悄放行（放行就是免费逃生）
#[test]
fn copy_missing_is_error_not_free_exit() {
    let mut m = machine();
    lock_with_attempts(&mut m, 3);

    for _ in 0..2 {
        let index = correct_index(m.session().unwrap().round.as_ref().unwrap());
        m.transition(
            SessionEvent::SubmitChoice {
                choice: index,
                copy: Some(copy_task()),
            },
            700_000,
        )
        .expect("答对");
    }

    let index = correct_index(m.session().unwrap().round.as_ref().unwrap());
    let err = m
        .transition(
            SessionEvent::SubmitChoice {
                choice: index,
                copy: None,
            },
            800_000,
        )
        .unwrap_err();

    assert_eq!(err, SessionError::CopyMissing);
    assert_eq!(m.state(), SessionState::Challenge, "不得因此出关");
    // 回归：报错时题目必须还在，否则会留下"状态 Challenge、题目已空"的死态
    assert!(
        m.session().unwrap().round.is_some(),
        "报错后题目不得被清空（过去的顺序会留下无题可答的死态）"
    );
    assert_eq!(
        m.transition(
            SessionEvent::SubmitChoice {
                choice: 0,
                copy: Some(copy_task()),
            },
            800_000
        )
        .map(|_| ()),
        Ok(()),
        "补上原文后应能正常继续，而不是一直 E_INVALID_CHOICE"
    );
}

#[test]
fn copying_passes_with_exact_text() {
    let mut m = machine();
    lock_with_attempts(&mut m, 3);
    m.session.as_mut().unwrap().copy = Some(copy_task());
    m.state = SessionState::Copying;

    let outcome = m
        .transition(
            SessionEvent::SubmitCopy {
                text: "道友且坐，莫问几时。".to_string(),
            },
            900_000,
        )
        .expect("抄对");

    match outcome {
        Outcome::Settled { result, .. } => assert_eq!(result, SessionResult::Early),
        other => panic!("应结算为 early，实际 {other:?}"),
    }
}

#[test]
fn copying_mismatch_reports_index() {
    let mut m = machine();
    lock_with_attempts(&mut m, 3);
    m.session.as_mut().unwrap().copy = Some(copy_task());
    m.state = SessionState::Copying;

    let outcome = m
        .transition(
            SessionEvent::SubmitCopy {
                text: "道友且座，莫问几时。".to_string(),
            },
            900_000,
        )
        .expect("抄错不是非法事件");

    match outcome {
        Outcome::CopyMismatch { index } => assert_eq!(index, 3),
        other => panic!("应报错字位置，实际 {other:?}"),
    }
    assert_eq!(m.state(), SessionState::Copying, "抄错留在抄写");
}

#[test]
fn emergency_requires_exact_sentence() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestEmergency, 600_000)
        .expect("进确认");

    let err = m
        .transition(
            SessionEvent::ConfirmEmergency {
                text: "我确认放弃本次闭关。".to_string(),
            },
            600_000,
        )
        .unwrap_err();

    assert_eq!(err, SessionError::ConfirmMismatch);
    assert_eq!(m.state(), SessionState::Confirming);
    assert_eq!(m.session().unwrap().faults, 0, "没确认就不该记过");
}

/// A-8：确认 → emergency，破戒 +1
#[test]
fn emergency_confirm_settles_and_counts_fault() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestEmergency, 600_000)
        .expect("进确认");

    let outcome = m
        .transition(
            SessionEvent::ConfirmEmergency {
                text: EMERGENCY_SENTENCE.to_string(),
            },
            600_000,
        )
        .expect("确认");

    match outcome {
        Outcome::Settled { result, session } => {
            assert_eq!(result, SessionResult::Emergency);
            assert_eq!(session.faults, 1);
        }
        other => panic!("应结算为 emergency，实际 {other:?}"),
    }
}

#[test]
fn cancel_emergency_returns_to_locked() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestEmergency, 600_000)
        .expect("进确认");

    let outcome = m
        .transition(SessionEvent::CancelEmergency, 600_000)
        .expect("取消");
    assert!(matches!(outcome, Outcome::Locked));
    assert_eq!(m.state(), SessionState::Locked);
    assert_eq!(m.session().unwrap().faults, 0, "取消不消耗成本");
}

/// A-7 / 非法转移：不在 Locked 时不许发起破戒（Esc 判定在前端之外）
#[test]
fn emergency_only_from_locked() {
    let mut m = machine();
    lock_with_attempts(&mut m, 1);

    let err = m.transition(SessionEvent::RequestEmergency, 0).unwrap_err();
    assert_eq!(err, SessionError::StateIllegal);
}

// ---------- 全局到点规则：五个活跃态各一条 ----------

fn assert_completed(mut m: SessionMachine, elapsed: u64) {
    let outcome = m.transition(SessionEvent::Tick, elapsed).expect("tick");
    match outcome {
        Outcome::Settled { result, .. } => assert_eq!(result, SessionResult::Completed),
        other => panic!("到点必须结算为 completed，实际 {other:?}"),
    }
    assert_eq!(m.state(), SessionState::Settled);
}

/// A-21：冷却中到点 → completed，不卡在冷却里
#[test]
fn deadline_during_cool_settles_completed() {
    let mut m = machine();
    arm_and_lock(&mut m, 600_000);
    m.transition(SessionEvent::RequestUnlock, 0).expect("解锁");
    assert_eq!(m.state(), SessionState::Cool);

    assert_completed(m, 600_000);
}

#[test]
fn deadline_during_challenge_settles_completed() {
    let mut m = machine();
    lock_with_attempts(&mut m, 1);
    assert_completed(m, HOUR_MS);
}

#[test]
fn deadline_during_copying_settles_completed() {
    let mut m = machine();
    lock_with_attempts(&mut m, 3);
    m.session.as_mut().unwrap().copy = Some(copy_task());
    m.state = SessionState::Copying;

    assert_completed(m, HOUR_MS);
}

#[test]
fn deadline_during_confirming_settles_completed() {
    let mut m = machine();
    arm_and_lock(&mut m, 600_000);
    m.transition(SessionEvent::RequestEmergency, 0)
        .expect("进确认");

    assert_completed(m, 600_000);
}

/// attempts 如实记录：冷却中到点时不清零（PRD §5.2 到点规则）
#[test]
fn deadline_keeps_attempts() {
    let mut m = machine();
    arm_and_lock(&mut m, 600_000);
    m.transition(SessionEvent::RequestUnlock, 0).expect("解锁");

    match m.transition(SessionEvent::Tick, 700_000).expect("tick") {
        Outcome::Settled { session, .. } => assert_eq!(session.attempts, 1),
        other => panic!("必须结算，实际 {other:?}"),
    }
}

// ---------- 非法转移：状态混用 ----------

#[test]
fn submit_choice_outside_challenge_is_illegal() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);

    let err = m
        .transition(
            SessionEvent::SubmitChoice {
                choice: 0,
                copy: None,
            },
            0,
        )
        .unwrap_err();
    assert_eq!(err, SessionError::StateIllegal);
}

#[test]
fn submit_copy_outside_copying_is_illegal() {
    let mut m = machine();
    lock_with_attempts(&mut m, 3);

    let err = m
        .transition(
            SessionEvent::SubmitCopy {
                text: "道友且坐，莫问几时。".to_string(),
            },
            0,
        )
        .unwrap_err();
    assert_eq!(err, SessionError::StateIllegal);
}

#[test]
fn request_unlock_outside_locked_is_illegal() {
    let mut m = machine();
    lock_with_attempts(&mut m, 1);

    let err = m.transition(SessionEvent::RequestUnlock, 0).unwrap_err();
    assert_eq!(err, SessionError::StateIllegal);
}

/// D-26：显示变化可以从冷却中出关（否则"冷却时拔屏"会卡在坏布局里）
#[test]
fn accident_allowed_from_cool() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);
    m.transition(SessionEvent::RequestUnlock, 600_000)
        .expect("解锁");

    let outcome = m
        .transition(
            SessionEvent::Finish {
                result: SessionResult::Accident,
            },
            700_000,
        )
        .expect("冷却中也能出关");

    match outcome {
        Outcome::Settled { result, .. } => assert_eq!(result, SessionResult::Accident),
        other => panic!("应结算为 accident，实际 {other:?}"),
    }
}

/// 不许用 Finish 伪造 early/emergency：那些必须走各自路径
#[test]
fn finish_cannot_forge_early() {
    let mut m = machine();
    arm_and_lock(&mut m, HOUR_MS);

    let err = m
        .transition(
            SessionEvent::Finish {
                result: SessionResult::Early,
            },
            1_000,
        )
        .unwrap_err();
    assert_eq!(err, SessionError::StateIllegal);
}
