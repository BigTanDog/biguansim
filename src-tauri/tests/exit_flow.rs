//! 退出流程的验收仿真：把 A-4 / A-5 / A-6 / A-8 / A-21 / A-22 / A-23 / A-25a
//! 的**逻辑层**用真实调用链跑一遍（真实题库加载 → 抽题 → 打乱 → 判定 → 成本 → 结算）。
//!
//! 这里能自动化的部分是"数字与状态对不对"；界面观感、Win32 拦键、输入法手感仍只能人工。
//! 想看过程表：`cargo test --test exit_flow -- --nocapture`。

use std::path::Path;

use biguansim_lib::core::bank::Bank;
use biguansim_lib::core::challenge::Round;
use biguansim_lib::core::copy::CopyTask;
use biguansim_lib::core::cost;
use biguansim_lib::core::rng::Rng;
use biguansim_lib::core::session::{
    Outcome, SessionError, SessionEvent, SessionMachine, SessionState, EMERGENCY_SENTENCE,
};
use biguansim_lib::core::store::SessionResult;
use biguansim_lib::events::PhasePayload;

const MINUTE: u64 = 60_000;
/// 题库里每题的正确项文本都写成「对」，因此可以从展示顺序里认出正确项——
/// 这正是用户在界面上看到的东西，测试不碰 `answer`。
const RIGHT: &str = "对";

/// 造一个 12 题的临时题库（走真实的 `Bank::load`，包含校验与抽题）
fn fixture_bank(label: &str) -> (Bank, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "biguansim-flow-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(dir.join("bank")).unwrap();

    let mut questions = Vec::new();
    for index in 0..12 {
        questions.push(format!(
            r#"{{"id":"q{index}","type":"choice","difficulty":"undergrad","source":"generated",
               "stem":"第 {index} 题","options":["{RIGHT}","甲","乙","丙"],"answer":0}}"#
        ));
    }
    let json = format!(
        r#"{{"version":1,"id":"fixture","name":"夹具阵法","questions":[{}]}}"#,
        questions.join(",")
    );
    std::fs::write(dir.join("bank/seed.json"), json.as_bytes()).unwrap();

    (Bank::load(&dir), dir)
}

fn start_locked(machine: &mut SessionMachine, minutes: u64) {
    machine
        .transition(
            SessionEvent::Start {
                duration_ms: minutes * MINUTE,
                now_ms: 1_760_875_920_000,
                now_mono_ms: 1_000,
                goal: Some("验收仿真".to_string()),
                bank: "fixture".to_string(),
                cooldown_base: 5,
                question_base: 3,
                // 仿真按正式版口径（不缩放），断言里的 5/10/15 分钟才对得上
                cost_scale: 1,
            },
            0,
        )
        .expect("入关");
    machine
        .transition(SessionEvent::ArmFinished, 0)
        .expect("起势结束");
    assert_eq!(machine.state(), SessionState::Locked);
}

/// 冷却到点 → 抽题 → Challenge，返回本轮题数
fn enter_challenge(machine: &mut SessionMachine, bank: &Bank, elapsed: u64) -> usize {
    match machine
        .transition(SessionEvent::Tick, elapsed)
        .expect("tick")
    {
        Outcome::CooldownElapsed => {}
        other => panic!("冷却应到点，实际 {other:?}"),
    }

    let attempts = machine.session().unwrap().attempts;
    let base = machine.session().unwrap().question_base;
    let needed = cost::question_count(attempts, base);
    let mut rng = Rng::new(7);
    let drawn = bank.draw(needed as usize, &mut rng);
    let round = Round::prepare(drawn, attempts, &mut rng);

    let outcome = machine
        .transition(SessionEvent::CoolFinished { round }, elapsed)
        .expect("进问阵");
    assert!(matches!(outcome, Outcome::Challenge));

    let total = machine.session().unwrap().round.as_ref().unwrap().total();
    println!("  → 第 {attempts} 轮问阵：{total} 题（期望 {needed}）");
    assert_eq!(total as u32, needed);
    total
}

fn correct_index(machine: &SessionMachine) -> usize {
    machine
        .session()
        .unwrap()
        .round
        .as_ref()
        .unwrap()
        .view()
        .unwrap()
        .options
        .iter()
        .position(|text| text == RIGHT)
        .expect("正确项一定在展示里")
}

fn wrong_index(machine: &SessionMachine) -> usize {
    (correct_index(machine) + 1) % 4
}

fn copy_task_for(attempts: u32) -> CopyTask {
    // 用真实语料的抽取逻辑，避免测试自己编一段"刚好 100 字"的假数据
    let book = biguansim_lib::core::quote::QuoteBook::load(Path::new("不存在的目录"));
    let text = book
        .copy_line(cost::copy_chars(attempts), &mut Rng::new(11))
        .expect("内嵌语料必有抄写段");
    CopyTask::new(text)
}

fn choose(machine: &mut SessionMachine, index: usize, elapsed: u64) -> Outcome {
    let copy = if machine.copy_chars() > 0 {
        Some(copy_task_for(machine.session().unwrap().attempts))
    } else {
        None
    };
    machine
        .transition(
            SessionEvent::SubmitChoice {
                choice: index,
                copy,
            },
            elapsed,
        )
        .expect("提交选项")
}

fn answer_all(machine: &mut SessionMachine, elapsed: u64) -> Outcome {
    loop {
        let total = machine.session().unwrap().round.as_ref().unwrap().total();
        let index = correct_index(machine);
        let outcome = choose(machine, index, elapsed);
        if let Outcome::ChallengeAdvanced = outcome {
            let next = machine.session().unwrap().round.as_ref().unwrap().total();
            assert_eq!(next, total);
            continue;
        }
        return outcome;
    }
}

/// A-5 + A-6 + A-23：连错两次后，第 3 轮全对必须进抄写（100 字），抄错不许放行
#[test]
fn wrong_answers_raise_cost_then_third_round_requires_copying() {
    let (bank, dir) = fixture_bank("cost");
    let mut machine = SessionMachine::new();
    start_locked(&mut machine, 60);

    // 第 1 轮：5 分钟 / 3 题 / 答错
    let elapsed = 10 * MINUTE;
    match machine
        .transition(SessionEvent::RequestUnlock, elapsed)
        .expect("解锁")
    {
        Outcome::Cool {
            attempt,
            cooldown_ms,
        } => {
            assert_eq!(attempt, 1);
            assert_eq!(cooldown_ms, 5 * MINUTE);
            println!("A-4  第 1 次解锁：冷却 5 分钟 ✓");
        }
        other => panic!("应进冷却，实际 {other:?}"),
    }

    enter_challenge(&mut machine, &bank, 15 * MINUTE);
    let wrong = wrong_index(&machine);
    match choose(&mut machine, wrong, 15 * MINUTE) {
        Outcome::Cool {
            attempt,
            cooldown_ms,
        } => {
            assert_eq!(attempt, 2);
            assert_eq!(cooldown_ms, 10 * MINUTE);
            println!("A-5  答错后第 2 次：冷却 10 分钟 ✓");
        }
        other => panic!("答错应回冷却，实际 {other:?}"),
    }

    // 第 2 轮：10 分钟 / 4 题 / 再答错
    enter_challenge(&mut machine, &bank, 25 * MINUTE);
    let wrong = wrong_index(&machine);
    match choose(&mut machine, wrong, 25 * MINUTE) {
        Outcome::Cool {
            attempt,
            cooldown_ms,
        } => {
            assert_eq!(attempt, 3);
            assert_eq!(cooldown_ms, 15 * MINUTE);
            println!("A-5  再答错第 3 次：冷却 15 分钟 ✓");
        }
        other => panic!("答错应回冷却，实际 {other:?}"),
    }

    // 第 3 轮：15 分钟 / 5 题 / 全对 → 抄写 100 字
    enter_challenge(&mut machine, &bank, 40 * MINUTE);
    match answer_all(&mut machine, 40 * MINUTE) {
        Outcome::Copying { target_chars } => {
            assert_eq!(target_chars, 100);
            println!("A-23 第 3 轮全对：进入抄写 100 字 ✓");
        }
        other => panic!("应进抄写，实际 {other:?}"),
    }
    assert_eq!(machine.state(), SessionState::Copying);

    // 抄错不许放行
    let task = machine.session().unwrap().copy.clone().expect("抄写任务");
    let mut broken: Vec<char> = task.text.chars().collect();
    broken[3] = '错';
    let broken: String = broken.into_iter().collect();
    match machine
        .transition(SessionEvent::SubmitCopy { text: broken }, 40 * MINUTE)
        .expect("抄错是合法事件")
    {
        Outcome::CopyMismatch { index } => {
            assert_eq!(index, 3);
            println!("A-23 抄错第 4 字：标红位置 3，留在抄写 ✓");
        }
        other => panic!("抄错应报位置，实际 {other:?}"),
    }
    assert_eq!(machine.state(), SessionState::Copying);

    // 抄对 → early
    match machine
        .transition(
            SessionEvent::SubmitCopy {
                text: task.text.clone(),
            },
            40 * MINUTE,
        )
        .expect("抄对")
    {
        Outcome::Settled { result, session } => {
            assert_eq!(result, SessionResult::Early);
            assert_eq!(session.attempts, 3);
            println!("A-6  逐字抄对：出关结果 early，attempts=3 ✓");
        }
        other => panic!("应结算，实际 {other:?}"),
    }

    let _ = std::fs::remove_dir_all(dir);
}

/// A-22：剩余不足一次冷却 → 拒绝，且**不消耗**尝试次数
#[test]
fn unlock_refused_when_remaining_is_short() {
    let mut machine = SessionMachine::new();
    start_locked(&mut machine, 25);

    let elapsed = 25 * MINUTE - 3 * MINUTE;
    match machine
        .transition(SessionEvent::RequestUnlock, elapsed)
        .expect("合法请求")
    {
        Outcome::UnlockRefused {
            need_ms,
            remaining_ms,
        } => {
            assert_eq!(need_ms, 5 * MINUTE);
            assert_eq!(remaining_ms, 3 * MINUTE);
            println!("A-22 剩余 3 分钟点解锁：拒绝（需 5 分钟）✓");
        }
        other => panic!("应被拒绝，实际 {other:?}"),
    }
    assert_eq!(machine.state(), SessionState::Locked);
    assert_eq!(machine.session().unwrap().attempts, 0, "被拒不得计成本");
}

/// A-21：冷却中到点 → completed，不卡在冷却里，attempts 如实记录
#[test]
fn deadline_during_cooldown_settles_completed() {
    let mut machine = SessionMachine::new();
    start_locked(&mut machine, 20);

    machine
        .transition(SessionEvent::RequestUnlock, 10 * MINUTE)
        .expect("解锁");
    assert_eq!(machine.state(), SessionState::Cool);

    match machine
        .transition(SessionEvent::Tick, 20 * MINUTE)
        .expect("tick")
    {
        Outcome::Settled { result, session } => {
            assert_eq!(result, SessionResult::Completed);
            assert_eq!(session.attempts, 1);
            println!("A-21 冷却中到点：自动 completed，attempts=1 ✓");
        }
        other => panic!("应结算，实际 {other:?}"),
    }
    assert_eq!(machine.state(), SessionState::Settled);
}

/// A-8：9 字必须逐字一致；正确后记 emergency 且破戒 +1
#[test]
fn emergency_needs_exact_sentence_and_counts_fault() {
    let mut machine = SessionMachine::new();
    start_locked(&mut machine, 60);

    machine
        .transition(SessionEvent::RequestEmergency, 10 * MINUTE)
        .expect("进确认");
    assert_eq!(machine.state(), SessionState::Confirming);

    let err = machine
        .transition(
            SessionEvent::ConfirmEmergency {
                text: format!("{EMERGENCY_SENTENCE}。"),
            },
            10 * MINUTE,
        )
        .unwrap_err();
    assert_eq!(err, SessionError::ConfirmMismatch);
    assert_eq!(machine.session().unwrap().faults, 0);
    println!("A-8  抄写多一个句号：E_CONFIRM_MISMATCH，不记过 ✓");

    match machine
        .transition(
            SessionEvent::ConfirmEmergency {
                text: EMERGENCY_SENTENCE.to_string(),
            },
            10 * MINUTE,
        )
        .expect("确认")
    {
        Outcome::Settled { result, session } => {
            assert_eq!(result, SessionResult::Emergency);
            assert_eq!(session.faults, 1);
            println!("A-8  9 字正确：emergency，faults=1 ✓");
        }
        other => panic!("应结算，实际 {other:?}"),
    }
}

/// A-25a：任何下发给前端的题面/载荷都不得包含 `answer`
#[test]
fn ipc_payloads_never_carry_the_answer() {
    let (bank, dir) = fixture_bank("payload");
    let mut rng = Rng::new(3);
    let round = Round::prepare(bank.draw(3, &mut rng), 1, &mut rng);
    let view = round.view().expect("有题");

    let view_json = serde_json::to_string(&view).unwrap();
    assert!(!view_json.contains("answer"), "{view_json}");
    println!("A-25a 题面载荷：{view_json}");

    let payload = PhasePayload::Challenge {
        id: view.id.clone(),
        index: view.index,
        total: view.total,
        stem: view.stem.clone(),
        options: view.options.clone(),
        attempt: 1,
    };
    let payload_json = serde_json::to_string(&payload).unwrap();
    assert!(!payload_json.contains("answer"), "{payload_json}");
    assert!(
        !payload_json.contains("answer_display"),
        "展示索引映射也不得外泄：{payload_json}"
    );

    let _ = std::fs::remove_dir_all(dir);
}

/// 成本表整体核对（PRD §7 的 1–4 次）
#[test]
fn cost_table_matches_prd() {
    let rows: Vec<(u32, u32, u32, u32)> = (1..=4)
        .map(|n| {
            (
                n,
                cost::cooldown_min(n, 5),
                cost::question_count(n, 3),
                cost::copy_chars(n),
            )
        })
        .collect();

    for (n, cool, questions, copy) in &rows {
        println!("n={n}: 冷却 {cool} 分钟 / {questions} 题 / 抄写 {copy} 字");
    }

    assert_eq!(
        rows,
        vec![
            (1, 5, 3, 0),
            (2, 10, 4, 0),
            (3, 15, 5, 100),
            (4, 20, 5, 200),
        ]
    );
}
