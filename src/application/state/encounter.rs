use super::{AppState, MAX_ENCOUNTER_OFFSET_FRAMES, MAX_FRAME};
use crate::application::state_types::{ArticleTimerStage, TimelineCandidate};
use crate::domain::rng::EncounterPlan;
use crate::domain::timing::{blink_ticks_to_seconds, game_frames_to_seconds, DEFAULT_FPS};
use crate::errors::{AppError, InputField};
use std::time::Instant;

impl AppState {
    // --- Target, Rotom, and production-plan calculations -----------------

    pub fn fps_value(&self) -> f64 {
        self.fps
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(DEFAULT_FPS)
    }

    pub fn rotom_threshold_value(&self) -> u64 {
        self.rotom_threshold
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|value| *value <= 100)
            .unwrap_or(79)
    }

    pub(super) fn pre_rotom_consumption_value(&self) -> i64 {
        self.pre_rotom_consumption
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|value| (0..=MAX_FRAME).contains(value))
            .filter(|_| self.consider_pre_rotom_consumption)
            .unwrap_or(0)
    }

    /// 候補の現在SFMT位置を返す。UIの実時間補正は位置へ加算しない。
    pub(super) fn candidate_position(&self, candidate_pos: usize) -> Option<i64> {
        let candidate = self.candidates.get(candidate_pos)?;
        let position = if candidate_pos == self.selected {
            self.next_blink_index.unwrap_or(candidate.sfmt_position)
        } else {
            candidate.sfmt_position
        };
        Some(position)
    }

    pub(super) fn clear_candidates(&mut self) {
        if !self.candidates.is_empty() {
            self.candidates.clear();
            self.candidate_generation = self.candidate_generation.wrapping_add(1);
        }
    }

    pub(super) fn replace_candidates(&mut self, candidates: Vec<TimelineCandidate>) {
        self.candidates = candidates;
        self.candidate_generation = self.candidate_generation.wrapping_add(1);
    }

    pub(super) fn selected_candidate_position(&self) -> Option<i64> {
        self.candidate_position(self.selected)
    }

    /// 現在選択中の閉じた位置に対応する、閾値判定上のお喋り予測。
    pub fn predicted_rotom_talk(&self) -> Option<bool> {
        let position = self.selected_candidate_position()?;
        self.cached_rotom_roll_at(
            self.search_config().ok()?.seed,
            position.saturating_add(self.pre_rotom_consumption_value()),
        )
        .map(|roll| u64::from(roll) < self.rotom_threshold_value())
    }

    /// 実機のお喋り結果を入力する。値はずれ補正と本番計算だけで使い、
    /// 設定したロトム確率や右上の○×案内は変更しない。
    pub fn set_observed_rotom_talk(&mut self, observed: bool) {
        self.observed_rotom_talk = Some(observed);
    }

    /// 開始位置のロール値と実機で確認したお喋り有無から、ロトム閾値の変更案を作る。
    ///
    /// ロトム判定はB押下位置の乱数値だけで決まり、NPCや本番Timelineの到達可否とは
    /// 独立している。`actual`は補正プレビューの共通呼び出し形を保つために受け取るが、
    /// ロトム補正値の計算には使用しない。
    pub(super) fn tolerance_value(&self) -> i64 {
        self.tolerance.trim().parse::<i64>().unwrap_or(5).max(0)
    }

    pub(super) fn encounter_offset_value(&self) -> i64 {
        if self.use_offset {
            self.encounter_offset.trim().parse::<i64>().unwrap_or(0)
        } else {
            0
        }
    }

    /// 本番へ移行した時点の予測を、補正画面で選択する初期値として保持する。
    /// 補正中に選択を変更しても、右上の到達可否計算は設定値を使い続ける。
    pub(super) fn initialize_observed_rotom_talk(&mut self) {
        if self.observed_rotom_talk.is_none() && self.consider_rotom_talk {
            self.observed_rotom_talk = self.predicted_rotom_talk();
        }
    }

    pub(super) fn plan_for_encounter(
        &self,
        seed: u32,
        observed_position: i64,
        offset: i64,
        consider_rotom: bool,
        threshold: u64,
        observed_talk: Option<bool>,
    ) -> Option<EncounterPlan> {
        let pre_rotom = self.pre_rotom_consumption_value();
        let b_position = observed_position.checked_add(pre_rotom)?;
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)?;
        let b_value = match cache.read_values(b_position, 1) {
            Ok(values) => values.first().copied()?,
            Err(error) => {
                self.defer_cache_io_error(error);
                return None;
            }
        };
        crate::domain::rng::encounter_plan_with_pre_rotom_value(
            observed_position,
            offset,
            consider_rotom,
            threshold,
            observed_talk,
            pre_rotom,
            self.npc_initial_load_value(),
            b_value,
        )
    }

    pub(super) fn npc_initial_load_value(&self) -> i64 {
        self.npc_initial_load
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|value| (0..=50).contains(value))
            .filter(|_| self.consider_npc_initial_load)
            .unwrap_or(0)
    }

    pub(super) fn encounter_offset_ticks(&self) -> Option<i64> {
        let frames = self.encounter_offset_value();
        (frames >= 0 && frames % 2 == 0).then_some(frames / 2)
    }

    pub fn encounter_offset_error(&self) -> Option<AppError> {
        if !self.use_offset {
            return None;
        }
        let trimmed = self.encounter_offset.trim();
        let frames = if trimmed.is_empty() {
            0
        } else if let Ok(frames) = trimmed.parse::<i64>() {
            frames
        } else {
            return Some(AppError::InvalidInteger(InputField::Offset));
        };
        if frames < 0 {
            return Some(AppError::NegativeValue(InputField::Offset));
        }
        if frames > MAX_ENCOUNTER_OFFSET_FRAMES {
            return Some(AppError::OutOfRange {
                field: InputField::Offset,
                min: 0,
                max: MAX_ENCOUNTER_OFFSET_FRAMES,
            });
        }
        if frames % 2 != 0 {
            return Some(AppError::OffsetMustBeEven);
        }
        None
    }

    /// B終了後からエンカウントまでの実時間。オフセットはFrame入力なので、
    /// 設定タブのFPSで秒へ換算する。
    pub(super) fn encounter_offset_seconds(&self) -> f64 {
        game_frames_to_seconds(
            self.encounter_offset_value().max(0) as f64,
            self.fps_value(),
        )
    }

    /// 本番タイムラインの1ステップは、外部ツール上では実機2F相当。
    /// `target_tick`はSFMT消費位置ではなく、時間経過ステップ数なので、
    /// TargetFrameの差分と同じように扱わない。
    pub(super) fn game_ticks_to_seconds(&self, ticks: i64) -> f64 {
        blink_ticks_to_seconds(ticks, self.fps_value())
    }

    pub(super) fn start_article_timer(&mut self, stage: ArticleTimerStage, seconds: f64) {
        self.article_timer_stage = Some(stage);
        self.article_timer_seconds = seconds.max(0.0);
        self.article_timer_start = Some(Instant::now());
        self.enter_countdown();
    }

    pub(super) fn clear_article_timer(&mut self) {
        self.article_timer_start = None;
        self.article_timer_seconds = 0.0;
        self.article_timer_stage = None;
        self.article_next_seconds = None;
    }

    pub fn article_plan_for_selected(&self) -> Option<EncounterPlan> {
        let config = self.search_config().ok()?;
        self.plan_for_encounter(
            config.seed,
            self.selected_candidate_position()?,
            self.encounter_offset_value(),
            self.consider_rotom_talk,
            self.rotom_threshold_value(),
            self.observed_rotom_talk,
        )
    }
}
