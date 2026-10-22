//! 平台能力层：只暴露能力（安装钩子、覆盖全屏、调光…），不认识 Core 的领域类型（TDD §1.2）。

pub mod display;
pub mod focus;
pub mod hook;
pub mod keyfilter;
pub mod log;
pub mod overlay;
pub mod power;
pub mod shell_guard;
pub mod sleep;
pub mod time;
