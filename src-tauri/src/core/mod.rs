//! 纯逻辑层：不依赖 `platform` 的具体实现，只依赖注入进来的 trait（TDD §1.2）。

pub mod bank;
pub mod bank_stats;
pub mod challenge;
pub mod copy;
pub mod cost;
pub mod quote;
pub mod rng;
pub mod session;
pub mod stats;
pub mod store;
pub mod timer;
