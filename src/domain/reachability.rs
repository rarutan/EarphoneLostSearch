//! 本番Timeline上の到達可否を判定する純粋な問い合わせ。

use super::rng::timeline_target;

/// 指定した開始位置からTargetへ、Timelineのステップ終点として厳密に
/// 着地できるかを返す。ステップ途中でTargetを通過する場合は`false`となる。
pub(crate) fn reaches_exactly(seed: u32, start: i64, target: i64, models: usize) -> bool {
    timeline_target(seed, start, target, models).is_some_and(|timing| timing.exact_tick.is_some())
}

#[cfg(test)]
mod tests {
    use super::reaches_exactly;

    #[test]
    fn distinguishes_exact_landing_from_crossing() {
        assert!(!reaches_exactly(1, 0, 1, 2));
        assert!(reaches_exactly(1, 0, 0, 2));
    }
}
