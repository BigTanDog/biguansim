//! 进程级共享状态。S7 之后由 session 模块接管生命周期，这里只放跨线程要用的标志。

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct AppState {
    locked: AtomicBool,
}

impl AppState {
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    pub fn set_locked(&self, value: bool) {
        self.locked.store(value, Ordering::SeqCst);
    }
}
