//! 纯逻辑层：不依赖 `platform` 的具体实现，只依赖注入进来的 trait（TDD §1.2）。

pub mod store;
pub mod timer;
