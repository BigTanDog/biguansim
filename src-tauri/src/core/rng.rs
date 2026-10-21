//! 可复现的伪随机数（xorshift64*）。
//!
//! 为什么不引 `rand`：这里只需要"抽题不重复 + 打乱选项 + 测试可复现"三件事。
//! `rand` 会带进 `getrandom` 与平台熵源一整套传递依赖，体积与审计成本都不划算
//! （M4 计划 D-28）。种子由调用方给，测试用固定种子，生产用 `from_entropy()`。

pub struct Rng {
    state: u64,
}

impl Rng {
    /// `seed = 0` 会让 xorshift 永远输出 0，这里强制置一位。
    pub fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    /// 生产用种子：纳秒时钟 + 进程 id。**不用于安全用途**，只用来避免每局抽到同一批题。
    pub fn from_entropy() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Self::new(nanos ^ ((std::process::id() as u64) << 32))
    }

    pub fn next_u64(&mut self) -> u64 {
        // xorshift64* 的标准移位量
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// `[0, bound)` 上的均匀整数。用拒绝采样去掉取模偏置（bound 很小时偏置可忽略，
    /// 但抽题次数多了仍是可测的偏差，索性一次做对）。
    pub fn below(&mut self, bound: u32) -> u32 {
        debug_assert!(bound > 0, "bound 必须为正");
        if bound <= 1 {
            return 0;
        }

        let bound = u64::from(bound);
        let zone = u64::MAX - (u64::MAX % bound);
        loop {
            let value = self.next_u64();
            if value < zone {
                return (value % bound) as u32;
            }
        }
    }

    /// Fisher-Yates 洗牌
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..8 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_diverges() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    /// 回归：`below` 必须落在界内，且大体均匀（四个桶各占 20% 上下）
    #[test]
    fn below_stays_in_range_and_is_roughly_uniform() {
        let mut rng = Rng::new(7);
        let mut buckets = [0usize; 4];
        for _ in 0..4_000 {
            let value = rng.below(4);
            assert!(value < 4);
            buckets[value as usize] += 1;
        }
        for count in buckets {
            assert!(count > 800, "分布偏了：{buckets:?}");
        }
    }

    #[test]
    fn shuffle_keeps_elements() {
        let mut rng = Rng::new(9);
        let mut items = vec![1, 2, 3, 4, 5];
        rng.shuffle(&mut items);
        items.sort_unstable();
        assert_eq!(items, vec![1, 2, 3, 4, 5]);
    }
}
