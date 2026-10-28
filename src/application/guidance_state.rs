//! 孵化タブの右上到達案内が使う計算結果と先読みデータを管理する。

use std::collections::{BTreeSet, HashMap};

/// 到達可否、次候補索引、計算済み範囲、間隔キャッシュを一体で保持する。
///
/// これらは同じ案内計算のライフサイクルで更新・破棄するため、画面全体の状態から分ける。
#[derive(Default)]
pub(crate) struct GuidanceProgress {
    reachability: HashMap<i64, bool>,
    reachable_frames: BTreeSet<i64>,
    unreachable_frames: BTreeSet<i64>,
    computed_until: Option<i64>,
    interval_base: Option<i64>,
    /// `prefix_seconds[n]` is the sum of frames `[base, base + n)`.
    prefix_seconds: Vec<f64>,
    session_generation: u64,
}

impl GuidanceProgress {
    pub(crate) fn invalidate(&mut self) {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.reachability.clear();
        self.reachable_frames.clear();
        self.unreachable_frames.clear();
        self.computed_until = None;
        self.interval_base = None;
        self.prefix_seconds.clear();
    }

    pub(crate) fn session_generation(&self) -> u64 {
        self.session_generation
    }

    pub(crate) fn record_reachability(&mut self, frame: i64, reachable: bool) {
        self.reachability.insert(frame, reachable);
        if reachable {
            self.reachable_frames.insert(frame);
            self.unreachable_frames.remove(&frame);
        } else {
            self.reachable_frames.remove(&frame);
            self.unreachable_frames.insert(frame);
        }
    }

    pub(crate) fn reachability(&self, frame: i64) -> Option<bool> {
        self.reachability.get(&frame).copied()
    }

    pub(crate) fn next_reachable(&self, start: i64, end: i64) -> Option<i64> {
        self.reachable_frames.range(start..=end).next().copied()
    }

    pub(crate) fn first_unreachable(&self, start: i64, end: i64) -> Option<i64> {
        self.unreachable_frames.range(start..=end).next().copied()
    }

    pub(crate) fn mark_computed_until(&mut self, frame: i64) {
        self.computed_until = Some(self.computed_until.unwrap_or(frame).max(frame));
    }

    pub(crate) fn known_end(&self, start: i64, end: i64) -> i64 {
        self.computed_until
            .filter(|frame| *frame >= start && *frame <= end)
            .unwrap_or(start.saturating_sub(1))
    }

    /// Add a contiguous worker batch. A gap rebases the summary to the new batch;
    /// this is safe because guidance only displays the current forward range.
    pub(crate) fn cache_interval_seconds(&mut self, start: i64, seconds: &[f64]) {
        if seconds.is_empty() {
            return;
        }
        let batch_end = start.saturating_add(seconds.len().saturating_sub(1) as i64);
        let prefix_end = self.interval_end();
        if self.interval_base.is_none()
            || prefix_end.is_some_and(|end| start > end.saturating_add(1))
            || self.interval_base.is_some_and(|base| batch_end < base)
        {
            self.interval_base = Some(start);
            self.prefix_seconds.clear();
            self.prefix_seconds.push(0.0);
        }
        let base = self.interval_base.unwrap_or(start);
        let append_from = self
            .interval_end()
            .map_or(base, |end| end.saturating_add(1))
            .max(start);
        if append_from > batch_end {
            return;
        }
        let offset = append_from.saturating_sub(start) as usize;
        for value in seconds.iter().copied().skip(offset) {
            let sum = self.prefix_seconds.last().copied().unwrap_or(0.0) + value;
            self.prefix_seconds.push(sum);
        }
    }

    fn interval_end(&self) -> Option<i64> {
        let base = self.interval_base?;
        let count = self.prefix_seconds.len().checked_sub(1)?;
        (count > 0).then(|| base.saturating_add(count.saturating_sub(1) as i64))
    }

    pub(crate) fn interval_seconds_sum(&self, start: i64, count: usize) -> Option<f64> {
        if count == 0 {
            return Some(0.0);
        }
        let base = self.interval_base?;
        let end_exclusive = start.checked_add(i64::try_from(count).ok()?)?;
        let cached_count = i64::try_from(self.prefix_seconds.len().checked_sub(1)?).ok()?;
        let cached_end_exclusive = base.checked_add(cached_count)?;
        if start < base || end_exclusive > cached_end_exclusive {
            return None;
        }
        let start_index = usize::try_from(start.checked_sub(base)?).ok()?;
        let end_index = usize::try_from(end_exclusive.checked_sub(base)?).ok()?;
        Some(self.prefix_seconds[end_index] - self.prefix_seconds[start_index])
    }

    pub(crate) fn interval_seconds_at(&self, frame: i64) -> Option<f64> {
        self.interval_seconds_sum(frame, 1)
    }
}

#[cfg(test)]
impl GuidanceProgress {
    pub(crate) fn is_empty(&self) -> bool {
        self.reachability.is_empty()
    }

    pub(crate) fn contains_reachable(&self, frame: i64) -> bool {
        self.reachable_frames.contains(&frame)
    }

    pub(crate) fn contains_unreachable(&self, frame: i64) -> bool {
        self.unreachable_frames.contains(&frame)
    }

    pub(crate) fn computed_until(&self) -> Option<i64> {
        self.computed_until
    }
}

#[cfg(test)]
mod tests {
    use super::GuidanceProgress;

    #[test]
    fn interval_prefix_sums_match_direct_ranges_and_rebase_on_a_gap() {
        let mut progress = GuidanceProgress::default();
        progress.cache_interval_seconds(10, &[0.25, 0.5, 1.0, 2.0]);

        assert_eq!(progress.interval_seconds_sum(10, 1), Some(0.25));
        assert_eq!(progress.interval_seconds_sum(11, 3), Some(3.5));
        assert_eq!(progress.interval_seconds_sum(10, 4), Some(3.75));
        assert_eq!(progress.interval_seconds_sum(9, 1), None);

        progress.cache_interval_seconds(20, &[3.0, 4.0]);
        assert_eq!(progress.interval_seconds_sum(10, 4), None);
        assert_eq!(progress.interval_seconds_sum(20, 2), Some(7.0));
    }

    #[test]
    fn interval_prefix_appends_contiguous_batches_and_ignores_overlap() {
        let mut progress = GuidanceProgress::default();
        progress.cache_interval_seconds(100, &[1.0, 2.0]);
        progress.cache_interval_seconds(102, &[3.0, 4.0]);
        assert_eq!(progress.interval_seconds_sum(100, 4), Some(10.0));

        progress.cache_interval_seconds(101, &[200.0, 300.0]);
        assert_eq!(progress.interval_seconds_sum(100, 4), Some(10.0));
    }

    #[test]
    fn unreachable_index_finds_first_failure_without_scanning_reachable_frames() {
        let mut progress = GuidanceProgress::default();
        for (frame, reachable) in [(20, true), (21, true), (22, false), (23, false)] {
            progress.record_reachability(frame, reachable);
        }
        assert_eq!(progress.first_unreachable(20, 23), Some(22));
        assert_eq!(progress.first_unreachable(23, 24), Some(23));
        assert_eq!(progress.first_unreachable(10, 19), None);
        assert!(progress.contains_unreachable(22));
    }
}
