use super::super::state_types::{CorrectionPreview, RotomCorrection, SearchConfig};
use super::{
    offset_after_observed_timeline_drift, AppState, CorrectionPreviewError, MIN_BLINK_INTERVALS,
};
#[cfg(test)]
use crate::application::state::MAX_FRAME;
use crate::domain::rng::EncounterPlan;
#[cfg(test)]
use crate::errors::InputField;
use crate::errors::{AppError, ArithmeticOperation, CacheError, StateIssue};

impl AppState {
    /// 現在のオフセットから、実際の到達Frameと設定TargetFrameのTimeline差分を
    /// 補正した新しいオフセットを求める。実着地が先なら差分を減算する。
    #[cfg(test)]
    pub fn suggested_offset_for_actual_from(&self, used_circle: i64) -> Result<i64, AppError> {
        let actual = self
            .actual_target_frame
            .trim()
            .parse::<i64>()
            .map_err(|_| AppError::InvalidInteger(InputField::ActualFrame))?;
        if !(0..=MAX_FRAME).contains(&actual) {
            return Err(AppError::OutOfRange {
                field: InputField::ActualFrame,
                min: 0,
                max: MAX_FRAME,
            });
        }
        let config = self.search_config()?;
        let delta_frames = self.timeline_drift_from_circle(&config, used_circle, actual)?;
        let current_offset = self.correction_source_offset_value();
        let corrected = offset_after_observed_timeline_drift(current_offset, delta_frames);
        // オフセットは0F未満にできないため、逆方向の補正が必要でも
        // ダイアログを無反応にせず、適用可能な下限へ丸めて返す。
        Ok(corrected.max(0))
    }

    /// 使用した開始○から本番開始位置を作り、Targetと実測Frameを同じ
    /// Timeline上で比較して表示Frame差分を返す。到達可否はプレビュー側で
    /// Target／実測それぞれに確認する。
    #[cfg(test)]
    pub(super) fn timeline_drift_from_circle(
        &self,
        config: &SearchConfig,
        used_circle: i64,
        actual: i64,
    ) -> Result<i64, AppError> {
        let plan = self.correction_plan(config, used_circle)?;
        self.timeline_drift_from_plan(config, &plan, actual)
    }

    pub(super) fn correction_plan(
        &self,
        config: &SearchConfig,
        used_circle: i64,
    ) -> Result<EncounterPlan, AppError> {
        // 補正欄で選択した実測のお喋り有無を本番Timelineへ反映する。
        // 未選択の場合だけ、開始位置の確率判定を既定値として使う。
        let source_rotom_talk = self
            .observed_rotom_talk
            .or_else(|| self.rotom_talk_at(used_circle));
        self.plan_for_encounter(
            config.seed,
            used_circle,
            0,
            self.consider_rotom_talk,
            self.rotom_threshold_value(),
            source_rotom_talk,
        )
        .ok_or(AppError::InvalidState(StateIssue::CalculationUnavailable))
    }

    pub(super) fn timeline_drift_from_plan(
        &self,
        config: &SearchConfig,
        plan: &EncounterPlan,
        actual: i64,
    ) -> Result<i64, AppError> {
        let target_frame = self.correction_target_frame.unwrap_or(config.target);
        // B+ロトムから続く同一Timeline上で両方のFrameを比較する。最速閉じオフセットを
        // 先に進めてModelStatusを再初期化すると`remain`を失い、オフセット前の実測Frameを
        // 照会できなくなるため、その経路は使わない。
        let start = plan.encounter_start;
        let target_timing = self
            .timeline_target_from_cache(config.seed, start, target_frame, config.npc_models)?
            .ok_or(AppError::InvalidState(StateIssue::CalculationUnavailable))?;
        let actual_timing = self
            .timeline_target_from_cache(config.seed, start, actual, config.npc_models)?
            .ok_or(AppError::InvalidState(StateIssue::CalculationUnavailable))?;
        let delta_frames = actual_timing
            .timeline_tick
            .checked_sub(target_timing.timeline_tick)
            .and_then(|value| value.checked_mul(2))
            .ok_or(AppError::ArithmeticOverflow(
                ArithmeticOperation::OffsetCalculation,
            ))?;
        Ok(delta_frames)
    }

    /// ずれ補正の確認ダイアログに表示する計算結果を作る。
    pub fn correction_preview_from(
        &self,
        used_circle: i64,
    ) -> Result<CorrectionPreview, CorrectionPreviewError> {
        self.correction_preview(used_circle)
    }

    pub(super) fn correction_preview(
        &self,
        used_circle: i64,
    ) -> Result<CorrectionPreview, CorrectionPreviewError> {
        let actual = self
            .actual_target_frame
            .trim()
            .parse::<i64>()
            .map_err(|_| CorrectionPreviewError::InvalidActualFrame)?;
        let config = self
            .search_config()
            .map_err(|_| CorrectionPreviewError::InvalidConfiguration)?;
        let current_offset = self.correction_source_offset_value();
        let rotom_position = used_circle
            .checked_add(self.pre_rotom_consumption_value())
            .ok_or(CorrectionPreviewError::CalculationUnavailable)?;
        self.ensure_correction_cache_readable(config.seed, rotom_position)?;
        let plan = self
            .correction_plan(&config, used_circle)
            .map_err(|_| CorrectionPreviewError::CalculationUnavailable)?;
        self.ensure_correction_cache_readable(config.seed, plan.encounter_start)?;
        let drift = self
            .timeline_drift_from_plan(&config, &plan, actual)
            .map_err(|error| match error {
                error @ AppError::Cache(CacheError::Io { .. }) => {
                    CorrectionPreviewError::CacheAccess(error)
                }
                _ => CorrectionPreviewError::CalculationUnavailable,
            })?;
        let suggested_offset = offset_after_observed_timeline_drift(current_offset, drift);
        // 逆方向のずれが現在のオフセットを超える場合も、補正操作自体
        // は発火させ、適用可能な下限0Fを提示する。
        let suggested_offset = suggested_offset.max(0);
        let rotom_correction = self.rotom_correction_from(used_circle, actual);
        // StartingFrameは3DSRNGToolへ入力する本番Timelineの起点であり、B後の
        // 時間オフセットは表示上の起点へ加えない。オフセットはTargetへ到達する
        // 経過時間・補正値として別に扱い、ロトムとNPC初期読み込みだけを起点へ反映する。
        let timeline_start = plan.encounter_start;
        let target_frame = self.correction_target_frame.unwrap_or(config.target);
        let target_on_timeline = self
            .timeline_target_from_cache(
                config.seed,
                timeline_start,
                target_frame,
                config.npc_models,
            )
            .map_err(CorrectionPreviewError::CacheAccess)?
            .is_some_and(|timing| timing.exact_tick.is_some());
        let actual_on_timeline = self
            .timeline_target_from_cache(config.seed, timeline_start, actual, config.npc_models)
            .map_err(CorrectionPreviewError::CacheAccess)?
            .is_some_and(|timing| timing.exact_tick.is_some());
        Ok(CorrectionPreview {
            suggested_offset,
            timeline_start,
            target_on_timeline,
            actual_on_timeline,
            actual_frame: actual,
            target_frame,
            rotom_correction,
        })
    }

    pub(super) fn ensure_correction_cache_readable(
        &self,
        seed: u32,
        position: i64,
    ) -> Result<(), CorrectionPreviewError> {
        let Some(cache) = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
        else {
            return Err(CorrectionPreviewError::CacheSeedMismatch);
        };
        if !(0..=cache.available_end()).contains(&position) {
            return Err(CorrectionPreviewError::CacheRangeUnavailable);
        }
        cache
            .verify_readable(position)
            .map_err(|error| CorrectionPreviewError::CacheAccess(AppError::from(error)))
    }

    pub fn correction_start_candidates(&self) -> Vec<i64> {
        let mut frames = if self.fastest_close {
            self.fast_close_circle_frames.clone()
        } else {
            self.correction_source_frame.into_iter().collect()
        };
        if self.fastest_close && frames.is_empty() {
            return frames;
        }
        // 追加待機方式では最速閉じ専用の履歴が残らないため、現在表示中の
        // ○を補正の開始候補として使う。現在のオフセットでTargetへ到達
        // できない場合でも、補正前の開始位置をダイアログで警告するために
        // 候補から除外しない。
        if frames.is_empty() {
            if let Some(current) = self.timeline_display_position() {
                frames.push(current);
            } else {
                // 候補が複数ある場合も、計算可能な開始位置を失わないように
                // 観測候補のSFMT位置をそのまま選択肢へ渡す。最速閉じで
                // ○が複数あるケースでは、ここで実際に使った位置を選べる。
                frames.extend(
                    self.candidates
                        .iter()
                        .map(|candidate| candidate.sfmt_position),
                );
            }
        }
        frames.sort_unstable();
        frames.dedup();
        frames
    }

    /// 補正欄で現在選択されている、B押下時点のSFMT位置を返す。
    pub fn selected_correction_start(&self) -> Option<i64> {
        let frames = self.correction_start_candidates();
        frames
            .get(
                self.correction_source_index
                    .min(frames.len().saturating_sub(1)),
            )
            .copied()
    }

    /// 補正欄の開始位置候補を選択する。選択位置は補正計算だけに使い、
    /// 右上の○×案内や観測候補そのものは変更しない。
    pub fn set_correction_source_index(&mut self, index: usize) -> bool {
        let frames = self.correction_start_candidates();
        let Some(&frame) = frames.get(index) else {
            return false;
        };
        self.correction_source_index = index;
        self.correction_source_frame = Some(frame);
        true
    }

    pub fn correction_source_index(&self) -> usize {
        self.correction_source_index
    }

    /// 補正欄の開始位置またはロトム閾値を変更したとき、ロトム欄の初期選択を
    /// その開始位置に対応する予測値へ同期する。
    pub fn sync_observed_rotom_to_correction_source(&mut self) -> bool {
        if !self.consider_rotom_talk {
            return false;
        }
        let Some(predicted) = self.predicted_rotom_talk_for_correction_source() else {
            return false;
        };
        let changed = self.observed_rotom_talk != Some(predicted);
        self.observed_rotom_talk = Some(predicted);
        changed
    }

    /// 補正欄で選択したB押下位置に対応するロトム判定を返す。
    pub fn predicted_rotom_talk_for_correction_source(&self) -> Option<bool> {
        let source = self.selected_correction_start()?;
        let config = self.search_config().ok()?;
        self.cached_rotom_roll_at(
            config.seed,
            source.saturating_add(self.pre_rotom_consumption_value()),
        )
        .map(|roll| u64::from(roll) < self.rotom_threshold_value())
    }

    /// ずれ補正を押した時点で、観測済みの入力から本番Timelineを準備する。
    /// 候補照合に必要な観測が不足している場合は何も作らず、呼び出し側で
    /// 補正元がない状態として扱う。
    pub fn prepare_correction_timeline(&mut self) {
        if self.candidates.is_empty()
            && self.observation_cache.is_some()
            && self.intervals.len() >= MIN_BLINK_INTERVALS
        {
            self.rematch_candidates();
            self.selected = 0;
        }
        if !self.candidates.is_empty() {
            self.recalculate_production();
            let _ = self.hatch_guidance();
        }
    }

    pub fn correction_inputs_complete(&self) -> bool {
        !self.actual_target_frame.trim().is_empty() && self.search_config().is_ok()
    }

    pub fn correction_timeline_needs_calculation(&self) -> bool {
        // 観測キャッシュが無いときだけ、補正ボタンから重い候補生成を
        // 始める。候補はあるがTargetが遠くてproductionだけ作れない場合も、
        // 補正ダイアログのTimeline差分計算は直接実行できる。
        self.candidates.is_empty() && self.observation_cache.is_none()
    }

    pub(super) fn rotom_correction_from(
        &self,
        used_circle: i64,
        _actual: i64,
    ) -> Option<RotomCorrection> {
        // 最速閉じでは、実際に開始した○の記録がない場合、補正元を
        // 推測した位置へロトム判定を適用しない。通常待機はEnter時点を
        // 呼び出し側から渡すため、この制限を受けない。
        let recorded_fastest_source = self.correction_source_frame == Some(used_circle)
            || self.fast_close_circle_frames.contains(&used_circle);
        if !self.consider_rotom_talk || (self.fastest_close && !recorded_fastest_source) {
            return None;
        }
        let config = self.search_config().ok()?;
        let roll = self.cached_rotom_roll_at(
            config.seed,
            used_circle.saturating_add(self.pre_rotom_consumption_value()),
        )?;
        let current_threshold = self.rotom_threshold_value();
        let predicted_talk = u64::from(roll) < current_threshold;
        let observed_talk = self.observed_rotom_talk?;
        if observed_talk == predicted_talk {
            return None;
        }
        let suggested_threshold = if observed_talk {
            u64::from(roll).saturating_add(1)
        } else {
            u64::from(roll)
        }
        .min(100);
        Some(RotomCorrection {
            roll,
            predicted_talk,
            observed_talk,
            current_threshold,
            suggested_threshold,
        })
    }

    pub(super) fn correction_source_offset_value(&self) -> i64 {
        self.correction_source_offset
            .unwrap_or_else(|| self.encounter_offset_value())
    }
}
