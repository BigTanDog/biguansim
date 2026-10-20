//! 显示变化检测（M3 S6）。
//!
//! 拔屏、改缩放、换主屏都会让"M 块屏"变成别的形状。策略：
//! 每 1 s 取一次快照，**连续 2 次**与基线不一致才判定成立（一次抖动不算），
//! 任一次回到基线即清零——避免把驱动瞬断、窗口重建误判成拔屏。

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tauri::AppHandle;

use crate::core::session::SessionState;
use crate::core::store::SessionResult;
use crate::runtime::{self, Runtime};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 连续几轮不一致才判定（TDD §4.5）
const CONFIRM_ROUNDS: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
struct DisplayRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    /// DPI 缩放，放大 100 倍存整数，避免浮点比较
    scale_percent: u32,
}

fn snapshot(app: &AppHandle) -> Option<Vec<DisplayRect>> {
    let monitors = app.available_monitors().ok()?;
    if monitors.is_empty() {
        return None;
    }

    let mut rects: Vec<DisplayRect> = monitors
        .iter()
        .map(|monitor| DisplayRect {
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width,
            height: monitor.size().height,
            scale_percent: (monitor.scale_factor() * 100.0).round() as u32,
        })
        .collect();

    // 枚举顺序不保证稳定，按坐标排序后再比较
    rects.sort_by_key(|rect| (rect.x, rect.y));
    Some(rects)
}

/// 连续不一致计数：一致就清零，不一致就 +1；满 `CONFIRM_ROUNDS` 才返回"应当出关"。
/// 抽成纯函数是为了让"一次抖动不算、连续两次才算"这条规则有单测（A-12 无法在本机实拨屏）。
fn advance(mismatches: u32, same: bool) -> (u32, bool) {
    if same {
        return (0, false);
    }

    let next = mismatches + 1;
    (next, next >= CONFIRM_ROUNDS)
}

/// 常驻线程：只在 Locked 期间工作，其余时间复位基线。
pub fn spawn(app: AppHandle, runtime: Arc<Runtime>) {
    thread::spawn(move || {
        let mut baseline: Option<Vec<DisplayRect>> = None;
        let mut mismatches: u32 = 0;

        loop {
            thread::sleep(POLL_INTERVAL);

            let locked = match runtime.machine.lock() {
                Ok(machine) => machine.state() == SessionState::Locked,
                Err(_) => false,
            };

            if !locked {
                baseline = None;
                mismatches = 0;
                continue;
            }

            let Some(current) = snapshot(&app) else {
                continue;
            };

            match &baseline {
                None => {
                    baseline = Some(current);
                    mismatches = 0;
                }
                Some(base) if *base == current => {
                    let (next, _) = advance(mismatches, true);
                    mismatches = next;
                }
                Some(base) => {
                    let (next, settle) = advance(mismatches, false);
                    mismatches = next;
                    if settle {
                        ::log::warn!(
                            "[display] 显示配置连续 {mismatches} 次与基线不一致（基线 {} 块屏，当前 {} 块屏），按 accident 出关",
                            base.len(),
                            current.len()
                        );
                        runtime::force_settle(&app, &runtime, SessionResult::Accident);
                        baseline = None;
                        mismatches = 0;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一次抖动不算：第一轮不一致只计数，不结算
    #[test]
    fn single_mismatch_does_not_settle() {
        let (count, settle) = advance(0, false);
        assert_eq!(count, 1);
        assert!(!settle, "一次不一致不得出关（A-12 要求连续 2 次）");
    }

    /// 连续两次才判定成立
    #[test]
    fn second_consecutive_mismatch_settles() {
        let (count, settle) = advance(1, false);
        assert_eq!(count, 2);
        assert!(settle);
    }

    /// 中途回到基线就清零，再接一次不一致也不能马上结算
    #[test]
    fn recovery_resets_counter() {
        let (count, settle) = advance(1, true);
        assert_eq!(count, 0);
        assert!(!settle);

        let (count, settle) = advance(count, false);
        assert_eq!(count, 1);
        assert!(!settle, "清零后重新计数，不应立即出关");
    }
}
