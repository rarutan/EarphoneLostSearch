//! 孵化タブの右上到達案内が使う計算結果と先読みデータを管理する。

use std::collections::{BTreeSet, HashMap};

/// 到達可否、次候補索引、計算済み範囲、間隔キャッシュを一体で保持する。
///
/// これらは同じ案内計算のライフサイクルで更新・破棄するため、画面全体の状態から分ける。
#[derive(Default)]
pub(crate) struct GuidanceProgress {
    reachability: HashMap<i64, bool>,
    reachable_frames: BTreeSet<i64>,
    computed_until: Option<i64>,
    interval_seconds: HashMap<i64, f64>,
    session_generation: u64,
}

impl GuidanceProgress {
    pub(crate) fn invalidate(&mut self) {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.reachability.clear();
        self.reachable_frames.clear();
        self.computed_until = None;
        self.interval_seconds.clear();
    }

    pub(crate) fn session_generation(&self) -> u64 {
        self.session_generation
    }

    pub(crate) fn record_reachability(&mut self, frame: i64, reachable: bool) {
        self.reachability.insert(frame, reachable);
        if reachable {
            self.reachable_frames.insert(frame);
        } else {
            self.reachable_frames.remove(&frame);
        }
    }

    pub(crate) fn reachability(&self, frame: i64) -> Option<bool> {
        self.reachability.get(&frame).copied()
    }

    pub(crate) fn next_reachable(&self, start: i64, end: i64) -> Option<i64> {
        self.reachable_frames.range(start..=end).next().copied()
    }

    pub(crate) fn mark_computed_until(&mut self, frame: i64) {
        self.computed_until = Some(self.computed_until.unwrap_or(frame).max(frame));
    }

    pub(crate) fn known_end(&self, start: i64, end: i64) -> i64 {
        self.computed_until
            .filter(|frame| *frame >= start && *frame <= end)
            .unwrap_or(start.saturating_sub(1))
    }

    pub(crate) fn cached_interval_seconds(&self, start: i64, count: usize) -> Option<Vec<f64>> {
        if count == 0 {
            return Some(Vec::new());
        }
        start.checked_add(count.saturating_sub(1) as i64)?;
        (0..count)
            .map(|offset| {
                self.interval_seconds
                    .get(&start.saturating_add(offset as i64))
                    .copied()
            })
            .collect()
    }

    pub(crate) fn cache_interval_seconds(&mut self, start: i64, seconds: &[f64]) {
        for (offset, seconds) in seconds.iter().copied().enumerate() {
            self.interval_seconds
                .insert(start.saturating_add(offset as i64), seconds);
        }
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

    pub(crate) fn computed_until(&self) -> Option<i64> {
        self.computed_until
    }
}
