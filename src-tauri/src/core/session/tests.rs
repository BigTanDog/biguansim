use super::*;

const NOW_MS: i64 = 1_760_875_920_000;

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
                result: SessionResult::Emergency,
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
