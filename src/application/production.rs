//! 本番区間の開始位置とTarget到達可否を問い合わせる。
//!
//! ロトム、NPC初期読み込み、オフセットの順序をここで一つの入力へまとめ、
//! 右上案内・候補判定・タイマーが同じ計算経路を共有できるようにする。

use crate::domain::rng::{
    advance_timeline, encounter_plan_with_pre_rotom_consumption, production_timeline,
};

/// Target到達判定に使う、UIから切り離した不変入力。
///
/// Encounter開始位置の計算とTarget判定は、右上案内・候補判定・表の判定で
/// 同じ条件を使う必要があるため、各呼び出し側で個別に組み立てない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProductionQuery {
    pub(crate) seed: u32,
    pub(crate) target: i64,
    pub(crate) models: usize,
    pub(crate) offset: i64,
    pub(crate) consider_rotom: bool,
    pub(crate) rotom_threshold: u64,
    pub(crate) observed_rotom_talk: Option<bool>,
    pub(crate) pre_rotom_consumption: i64,
    pub(crate) npc_initial_load: i64,
    pub(crate) fastest_ticks: Option<i64>,
}

/// 本番Timelineを同期計算してよいTargetまでの上限。
pub(crate) const MAX_PRODUCTION_LOOKAHEAD: i64 = 100_000;

impl ProductionQuery {
    /// B直後の追加消費・ロトム・初期読み込みを反映した本番開始位置。
    /// 最速閉じの指定ステップは、呼び出し側がこの位置から別途進める。
    pub(crate) fn base_encounter_start(self, observed_position: i64) -> Option<i64> {
        encounter_plan_with_pre_rotom_consumption(
            self.seed,
            observed_position,
            self.offset,
            self.consider_rotom,
            self.rotom_threshold,
            self.observed_rotom_talk,
            self.pre_rotom_consumption,
            self.npc_initial_load,
        )
        .map(|plan| plan.encounter_start)
    }

    pub(crate) fn encounter_start(self, observed_position: i64) -> Option<i64> {
        let start = self.base_encounter_start(observed_position)?;
        self.fastest_ticks.map_or(Some(start), |ticks| {
            advance_timeline(self.seed, start, self.models, ticks)
        })
    }

    pub(crate) fn target_reachable(self, observed_position: i64) -> bool {
        if self.target.saturating_sub(observed_position) > MAX_PRODUCTION_LOOKAHEAD {
            return false;
        }
        let Some(start) = self.encounter_start(observed_position) else {
            return false;
        };
        production_timeline(self.seed, start, self.target, self.models)
            .target_tick
            .is_some()
    }

    /// 表の○×判定用。最速閉じは、指定オフセット後の位置がTargetと
    /// 完全一致する必要があるため、通常待機の区間判定とは分ける。
    #[cfg(test)]
    pub(crate) fn target_exactly_reached(self, observed_position: i64) -> Option<bool> {
        if self.target.saturating_sub(observed_position) > MAX_PRODUCTION_LOOKAHEAD {
            return Some(false);
        }
        let start = self.encounter_start(observed_position)?;
        Some(if self.fastest_ticks.is_some() {
            start == self.target
        } else {
            production_timeline(self.seed, start, self.target, self.models)
                .target_tick
                .is_some()
        })
    }
}
