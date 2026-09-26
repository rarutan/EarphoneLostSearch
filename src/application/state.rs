//! 孵化タブを中心とするアプリケーション状態と状態遷移を管理する。
//!
//! 検索・補正・本番タイマーの計算結果をセッションへ適用し、UIが描画する
//! スナップショットを組み立てる。重い計算自体はドメイン層またはワーカーへ委譲する。

use super::guidance_state::GuidanceProgress;
use crate::application::defaults;
use crate::application::production::ProductionQuery;
#[cfg(test)]
pub(crate) use crate::application::timer_state::NormalTimerSegment;
pub(crate) use crate::application::timer_state::NormalTimerState;
#[cfg(test)]
use crate::domain::rng::target_reachability_range;
use crate::domain::rng::{
    advance_timeline_from_values, target_reachability_range_from_cache, timeline_target_from_cache,
    EncounterPlan, ObservationCache, ObservationMatch, ProductionTimeline, ProductionTimelineCache,
    PRODUCTION_TIMELINE_CACHE_FRAMES,
};
#[cfg(test)]
use crate::domain::rng::{encounter_plan_with_observed_rotom, rotom_roll_at};
use crate::domain::timing::{
    blink_ticks_to_seconds, game_frames_to_seconds, seconds_to_game_frames, DEFAULT_FPS,
};
use crate::errors::{AppError, ArithmeticOperation, CacheError, InputField, StateIssue};
#[cfg(test)]
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) use super::state_types::*;

pub const MAX_FRAME: i64 = crate::domain::rng::MAX_SFMT_FRAME;
/// NPC入力で許可するNPC数の上限（主人公は内部で別途1体加算）。
pub const MAX_NPC_COUNT: usize = 50;
const MAX_AUTO_TOLERANCE: i64 = 15;
pub(crate) const MIN_BLINK_INTERVALS: usize = 2;
/// 1回の観測セッションで記録できる瞬きの最大回数。
///
/// タイムライン仕様の実運用では数回〜十数回の観測で候補を絞れるため、
/// 64回（間隔63個）あれば十分な余裕を持ちつつ、誤操作による無制限な
/// 照合・メモリ増加を防げる。
pub(crate) const MAX_BLINK_OBSERVATIONS: usize = 64;
const MAX_TIMELINE_MODELS: usize = 256;
// 1回の描画で計算する案内範囲。検索全体は設定された最大5000万Frameまで
// 次の描画へ継続し、遠い○を固定上限で打ち切らない。
const GUIDANCE_CHUNK: i64 = 4096;
// 最速閉じの理論上の候補範囲に加える表示Frameの安全余白。
const FASTEST_CLOSE_SEARCH_BUFFER: i64 = 100;
/// 孵化タブの本番Timeline／右上○×案内で許容するTargetまでの距離。
/// 遠いTargetを指定した場合に、Target位置までのSFMT列を一括生成しない。
pub(crate) use crate::application::production::MAX_PRODUCTION_LOOKAHEAD;
/// 孵化タブで入力するB後オフセットの上限（表示Frame）。
pub const MAX_ENCOUNTER_OFFSET_FRAMES: i64 = 10_000;

#[derive(Clone, Copy)]
struct GuidanceScanPlan {
    config: SearchConfig,
    current: i64,
    search_start: i64,
    search_end: i64,
    known_end: i64,
    fastest_bounds: Option<(i64, i64, bool)>,
    fastest_ticks: Option<i64>,
    target_too_far: bool,
}

#[derive(Debug)]
pub(crate) enum CorrectionPreviewError {
    InvalidActualFrame,
    InvalidConfiguration,
    CacheSeedMismatch,
    CacheRangeUnavailable,
    CacheAccess(AppError),
    CalculationUnavailable,
}

impl std::fmt::Display for CorrectionPreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidActualFrame => {
                formatter.write_str("実際に出現したTarget Frameは整数で入力してください。")
            }
            Self::InvalidConfiguration => {
                formatter.write_str("補正に必要な検索条件を確認してください。")
            }
            Self::CacheSeedMismatch => {
                formatter.write_str("観測キャッシュが現在のSeedと一致しません。")
            }
            Self::CacheRangeUnavailable => {
                formatter.write_str("補正に必要な開始位置がキャッシュ範囲外です。")
            }
            Self::CacheAccess(error) => {
                write!(formatter, "SFMTキャッシュを読み込めません: {error}")
            }
            Self::CalculationUnavailable => {
                formatter.write_str("現在のTimelineから補正値を計算できません。")
            }
        }
    }
}

/// 選択中候補のTarget計算をUIスレッドから切り離すための入力。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProductionPreparationRequest {
    pub(crate) session_generation: u64,
    pub(crate) selected_candidate: usize,
    pub(crate) observed_position: i64,
    pub(crate) encounter_start: i64,
    pub(crate) query: ProductionQuery,
}

/// ワーカーが準備した本番Timelineと、そのTarget判定。
pub(crate) struct PreparedProduction {
    cache: ProductionTimelineCache,
    production: ProductionTimeline,
}

pub(crate) use crate::infrastructure::offset_store::{OffsetStore, SavedOffset};

pub struct AppState {
    mode: Mode,
    pub seed: String,
    pub range_start: String,
    pub range_end: String,
    pub range_before_target: String,
    pub target: String,
    /// TargetFrameの入力途中を保持し、確定操作まで重い再計算を遅延する。
    pub target_dirty: bool,
    pub actual_target_frame: String,
    pub npc: String,
    pub blank_frames: String,
    /// 記事方式のB終了後〜エンカウントまでの実測オフセット。
    pub encounter_offset: String,
    pub use_offset: bool,
    /// ロトムのお喋り判定に使う閾値。SFMT値%100がこの値未満ならお喋りあり。
    pub rotom_threshold: String,
    /// ロトムのお喋りによる追加消費を本番計算へ含めるか。
    pub consider_rotom_talk: bool,
    /// ロトム判定前に行う補正消費。チェックOFF時は0として扱う。
    pub consider_pre_rotom_consumption: bool,
    pub pre_rotom_consumption: String,
    /// 実機で確認したお喋り有無。Noneなら現在位置の閾値判定を使う。
    pub observed_rotom_talk: Option<bool>,
    /// ロトム判定後に行うフィールドNPC初期読み込みを考慮するか。
    pub consider_npc_initial_load: bool,
    /// NPC初期読み込みの消費数。チェックOFF時は0として扱う。
    pub npc_initial_load: String,
    /// チェック時は、B後に指定オフセット時間だけ通常Timelineを進めて即エンカウントする。
    pub fastest_close: bool,
    pub tolerance: String,
    /// 実機のFrameを秒へ換算するFPS。既定値は59.8621だが環境に合わせて変更できる。
    pub fps: String,
    pub beep_interval: String,
    pub beep_count: String,
    pub offset_store: OffsetStore,
    pub normal_timer: NormalTimerState,
    /// Seed単位の共有キャッシュ。開始位置ごとのタイムラインは保持しない。
    pub observation_cache: Option<ObservationCache>,
    /// UIスレッドの補助計算で検出したキャッシュI/O障害を、次の更新で表示する。
    deferred_cache_io_error: std::sync::Mutex<Option<AppError>>,
    /// 右上の到達案内が共有する結果・索引・先読み状態。
    guidance: GuidanceProgress,
    /// 観測で特定した孵化演出終了位置から計算した本番経路。
    pub production: Option<ProductionTimeline>,
    /// Target確定後のキャッシュ延長・本番Timeline計算中に案内へ表示する。
    pub production_preparation_pending: bool,
    /// 本番Timeline準備中に受け付けた移行操作。準備が完了したら実行する。
    encounter_start_requested: bool,
    /// 確定した本番開始位置から先の連続Timeline。Targetはこのキャッシュへ
    /// 検索するだけで、Targetを変更してもTimelineを作り直さない。
    production_cache: Option<ProductionTimelineCache>,
    /// 観測列に一致した候補。各候補が1つの乱数列上の1つの観測位置を表す。
    pub candidates: Vec<TimelineCandidate>,
    pub observed: Vec<Instant>,
    /// 観測した瞬き間隔（FieldTimelineと同じ表示Frame単位）。
    pub intervals: Vec<i64>,
    pub match_tolerance: i64,
    pub search_exhausted: bool,
    pub selected: usize,
    timer_start: Option<Instant>,
    timer_seconds: f64,
    article_timer_start: Option<Instant>,
    article_timer_seconds: f64,
    pub article_timer_stage: Option<ArticleTimerStage>,
    /// オフセット計時中に左上の`next≫`へ表示し、終了後に開始する本番待機時間。
    pub article_next_seconds: Option<f64>,
    next_blink: Option<Instant>,
    next_blink_index: Option<i64>,
    /// 表で追従する現在位置。観測後は現在行を画面内に維持する。
    pub table_follow_position: Option<i64>,
    /// 実機で最速閉じに使用できた○Frame。ずれ補正時の開始候補に使う。
    pub fast_close_circle_frames: Vec<i64>,
    /// 本番遷移／ずれ検証入口を押した瞬間の補正元とTarget。
    correction_source_frame: Option<i64>,
    /// ずれ補正欄で選択している、実機のB押下位置の候補番号。
    correction_source_index: usize,
    correction_target_frame: Option<i64>,
    /// ずれ補正を繰り返す場合も変化させない、本番移行時点のオフセット。
    correction_source_offset: Option<i64>,
    /// 予測音の実時間Frame補正。SFMT位置とタイムライン自体は変更しない。
    pub adjust: i64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mode::Idle,
            seed: defaults::SEED.into(),
            range_start: defaults::HATCH_RANGE_START.into(),
            range_end: defaults::HATCH_RANGE_END.into(),
            range_before_target: defaults::HATCH_RANGE_BEFORE_TARGET.into(),
            target: defaults::HATCH_TARGET.into(),
            target_dirty: false,
            actual_target_frame: String::new(),
            npc: defaults::HATCH_NPC.into(),
            blank_frames: defaults::HATCH_BLANK_FRAMES.into(),
            encounter_offset: defaults::HATCH_ENCOUNTER_OFFSET.into(),
            use_offset: defaults::DEFAULT_USE_OFFSET,
            rotom_threshold: defaults::HATCH_ROTOM_THRESHOLD.into(),
            consider_rotom_talk: defaults::DEFAULT_CONSIDER_ROTOM_TALK,
            consider_pre_rotom_consumption: false,
            pre_rotom_consumption: defaults::HATCH_PRE_ROTOM_CONSUMPTION.into(),
            observed_rotom_talk: None,
            consider_npc_initial_load: defaults::DEFAULT_CONSIDER_NPC_INITIAL_LOAD,
            npc_initial_load: defaults::HATCH_NPC_INITIAL_LOAD.into(),
            fastest_close: defaults::DEFAULT_FASTEST_CLOSE,
            tolerance: defaults::HATCH_TOLERANCE.into(),
            fps: defaults::DEFAULT_FPS_TEXT.into(),
            beep_interval: defaults::DEFAULT_BEEP_INTERVAL.into(),
            beep_count: defaults::DEFAULT_BEEP_COUNT.into(),
            offset_store: OffsetStore::load(),
            normal_timer: NormalTimerState::default(),
            observation_cache: None,
            deferred_cache_io_error: std::sync::Mutex::new(None),
            guidance: GuidanceProgress::default(),
            production: None,
            production_preparation_pending: false,
            encounter_start_requested: false,
            production_cache: None,
            candidates: Vec::new(),
            observed: Vec::new(),
            intervals: Vec::new(),
            match_tolerance: 5,
            search_exhausted: false,
            selected: 0,
            timer_start: None,
            timer_seconds: 0.0,
            article_timer_start: None,
            article_timer_seconds: 0.0,
            article_timer_stage: None,
            article_next_seconds: None,
            next_blink: None,
            next_blink_index: None,
            table_follow_position: None,
            fast_close_circle_frames: Vec::new(),
            correction_source_frame: None,
            correction_source_index: 0,
            correction_target_frame: None,
            correction_source_offset: None,
            adjust: defaults::DEFAULT_ADJUST,
        }
    }
}

impl AppState {
    // --- Configuration, validation, and correction -----------------------

    /// UIと検索Workerが共有するセッションフェーズを返す。
    pub fn session_phase(&self) -> SessionPhase {
        match self.mode {
            Mode::Idle => SessionPhase::Input,
            Mode::Observing => SessionPhase::Observing,
            Mode::Ready => SessionPhase::TimelineReady,
            Mode::Production | Mode::Countdown | Mode::Encounter => SessionPhase::Timer,
            Mode::Finished => SessionPhase::ProductionFinished,
            Mode::Correction => SessionPhase::Correction,
        }
    }

    /// 内部の詳細モードをUIが直接参照せずに確認するための問い合わせ。
    ///
    /// `SessionPhase`は画面上の意味を表し、これらの問い合わせはbeepや
    /// タイマー段階のような孵化タブ固有の分岐に使う。
    pub fn is_idle(&self) -> bool {
        self.mode == Mode::Idle
    }

    pub fn is_observing(&self) -> bool {
        self.mode == Mode::Observing
    }

    pub fn is_ready(&self) -> bool {
        self.mode == Mode::Ready
    }

    pub fn is_countdown(&self) -> bool {
        self.mode == Mode::Countdown
    }

    pub fn is_production(&self) -> bool {
        self.mode == Mode::Production
    }

    pub fn is_encounter(&self) -> bool {
        self.mode == Mode::Encounter
    }

    pub fn is_finished(&self) -> bool {
        self.mode == Mode::Finished
    }

    pub fn is_blink_phase(&self) -> bool {
        matches!(self.mode, Mode::Idle | Mode::Observing | Mode::Ready)
    }

    pub fn is_timer_phase(&self) -> bool {
        matches!(
            self.mode,
            Mode::Countdown | Mode::Encounter | Mode::Production
        )
    }

    pub fn timer_start_time(&self) -> Option<Instant> {
        self.timer_start
    }

    pub fn timer_seconds_value(&self) -> f64 {
        self.timer_seconds
    }

    pub fn article_timer_start_time(&self) -> Option<Instant> {
        self.article_timer_start
    }

    pub fn article_timer_seconds_value(&self) -> f64 {
        self.article_timer_seconds
    }

    pub fn next_blink_deadline(&self) -> Option<Instant> {
        self.next_blink
    }

    pub fn has_next_blink(&self) -> bool {
        self.next_blink.is_some()
    }

    fn enter_idle(&mut self) {
        self.mode = Mode::Idle;
    }

    fn enter_observing(&mut self) {
        self.mode = Mode::Observing;
    }

    fn enter_ready(&mut self) {
        self.mode = Mode::Ready;
    }

    fn enter_countdown(&mut self) {
        self.mode = Mode::Countdown;
    }

    fn enter_finished(&mut self) {
        self.mode = Mode::Finished;
    }

    fn enter_correction(&mut self) {
        self.mode = Mode::Correction;
    }

    pub fn can_edit_general_settings(&self) -> bool {
        matches!(
            self.session_phase(),
            SessionPhase::Input | SessionPhase::Observing | SessionPhase::TimelineReady
        )
    }

    /// 瞬き観測の範囲・許容差などを編集できるのは、観測開始前だけに限定する。
    /// 観測後は入力済みの条件と候補を固定し、やり直しはリセットから行う。
    pub fn can_edit_observation_settings(&self) -> bool {
        self.session_phase() == SessionPhase::Input
    }

    pub fn can_edit_correction_inputs(&self) -> bool {
        matches!(
            self.session_phase(),
            SessionPhase::ProductionFinished | SessionPhase::Correction
        )
    }

    pub fn can_start_timer(&self) -> bool {
        self.session_phase() == SessionPhase::TimelineReady
    }

    pub fn can_enter_correction(&self) -> bool {
        self.session_phase() == SessionPhase::ProductionFinished
    }

    /// 最速閉じの補正入口を実行できるのは、記録済みの○がある場合だけ。
    /// ○以外の現在位置をB押下位置として保存すると、補正候補とTarget到達判定が食い違う。
    pub fn can_finish_fastest_close_for_correction(&self) -> bool {
        if self.mode != Mode::Ready || !self.fastest_close {
            return false;
        }
        !self.fast_close_circle_frames.is_empty()
    }

    pub fn finish_production(&mut self) {
        if self.mode == Mode::Production {
            self.enter_finished();
            self.clear_article_timer();
        }
    }

    /// Productionタイマーの期限を確認し、経過済みなら完了状態へ遷移する。
    /// UIと仮想時刻テストが同じ判定を使う。
    pub(crate) fn finish_production_if_elapsed(&mut self, now: Instant) -> bool {
        if self.mode != Mode::Production || self.remaining_at(now) > 0.0 {
            return false;
        }
        self.finish_production();
        true
    }

    /// 補正モードへ明示的に遷移する。以後はリセットまで通常タイマーへ
    /// 戻らない。ダイアログの表示や計算は呼び出し側が担当する。
    pub fn enter_correction_mode(&mut self) {
        self.enter_correction();
        self.timer_start = None;
        self.next_blink = None;
        self.clear_article_timer();
    }

    fn clear_guidance_reachability(&mut self) {
        self.guidance.invalidate();
    }

    fn record_guidance_reachability(&mut self, frame: i64, reachable: bool) {
        self.guidance.record_reachability(frame, reachable);
    }

    /// 右上案内で使う瞬き間隔を区間単位で取得する。
    ///
    /// 呼び出し元のバックグラウンド処理が要求位置までSFMTキャッシュを延長してから
    /// 呼ぶ。描画中にキャッシュ終端を越えても、ここではSeedから再生成しない。
    fn guidance_interval_seconds_range(
        &mut self,
        cache: &ObservationCache,
        start: i64,
        count: usize,
    ) -> Result<Option<Vec<f64>>, AppError> {
        if start < 0 {
            return Ok(None);
        }
        if count == 0 {
            return Ok(Some(Vec::new()));
        }
        let Some(end) = start.checked_add(count.saturating_sub(1) as i64) else {
            return Err(AppError::ArithmeticOverflow(
                ArithmeticOperation::CacheRange,
            ));
        };
        if let Some(cached) = self.guidance.cached_interval_seconds(start, count) {
            return Ok(Some(cached));
        }

        if end > cache.available_end() {
            return Ok(None);
        }
        let seconds = cache.interval_seconds_range(start, count)?;
        self.guidance.cache_interval_seconds(start, &seconds);
        Ok(Some(seconds))
    }

    fn guidance_known_end(&self, start: i64, end: i64) -> i64 {
        self.guidance.known_end(start, end)
    }

    /// 右上案内の全経路で共有する検索範囲と設定スナップショットを作る。
    ///
    /// `guidance_search_request`、`hatch_guidance`、`guidance_search_pending`が
    /// 個別に範囲を組み立てると、Workerが調べた範囲と表示が参照する範囲が
    /// 食い違う。ここではmainと同じ境界を一度だけ計算し、3経路から共有する。
    fn guidance_scan_plan(&self) -> Option<GuidanceScanPlan> {
        if self.encounter_offset_error().is_some() {
            return None;
        }
        let current = self.timeline_display_position()?;
        let config = self.search_config().ok()?;
        let fastest_bounds = self
            .fastest_close
            .then(|| self.fastest_close_guidance_bounds(&config))
            .flatten();
        let fastest_ticks = self
            .fastest_close
            .then(|| self.encounter_offset_ticks())
            .flatten();
        if self.fastest_close && fastest_ticks.is_none() {
            return None;
        }
        if config.target.saturating_sub(current) > MAX_PRODUCTION_LOOKAHEAD {
            return Some(GuidanceScanPlan {
                config,
                current,
                search_start: current,
                search_end: current.saturating_sub(1),
                known_end: current.saturating_sub(1),
                fastest_bounds,
                fastest_ticks,
                target_too_far: true,
            });
        }
        let theoretical_start = fastest_bounds.map(|bounds| bounds.0).unwrap_or(current);
        let search_start = current.max(theoretical_start);
        // Targetを観測範囲の暗黙の延長として扱わず、main側と同じ式で上限を求める。
        let configured_end = config
            .range_end
            .max(current)
            .min(MAX_FRAME)
            .min(current.saturating_add(MAX_PRODUCTION_LOOKAHEAD));
        let search_end = fastest_bounds
            .map(|bounds| {
                // 最速閉じはTargetの理論範囲を検索する。観測範囲の終点が
                // 現在地より前へ進んでいても、Targetが現在地から10万F以内
                // なら、理論範囲を観測範囲終点で切り詰めない。
                configured_end
                    .max(bounds.1)
                    .min(current.saturating_add(MAX_PRODUCTION_LOOKAHEAD))
                    .min(MAX_FRAME)
            })
            .unwrap_or(configured_end);
        let known_end = self.guidance_known_end(search_start, search_end);
        Some(GuidanceScanPlan {
            config,
            current,
            search_start,
            search_end,
            known_end,
            fastest_bounds,
            fastest_ticks,
            target_too_far: false,
        })
    }

    pub fn search_config(&self) -> Result<SearchConfig, AppError> {
        let seed = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16)
            .map_err(|_| AppError::InvalidSeed)?;
        let range_start = self
            .range_start
            .trim()
            .parse::<i64>()
            .map_err(|_| AppError::InvalidInteger(InputField::SearchStart))?;
        let range_end = self
            .range_end
            .trim()
            .parse::<i64>()
            .map_err(|_| AppError::InvalidInteger(InputField::SearchEnd))?;
        let target = self
            .target
            .trim()
            .parse::<i64>()
            .map_err(|_| AppError::InvalidInteger(InputField::TargetFrame))?;
        if range_start < 0
            || range_end < range_start
            || target < 0
            || range_end > MAX_FRAME
            || target > MAX_FRAME
        {
            return Err(AppError::OutOfRange {
                field: InputField::SearchStart,
                min: 0,
                max: MAX_FRAME,
            });
        }
        let npc_count = self
            .npc
            .trim()
            .parse::<usize>()
            .map_err(|_| AppError::InvalidInteger(InputField::NpcCount))?;
        if npc_count > MAX_NPC_COUNT {
            return Err(AppError::OutOfRange {
                field: InputField::NpcCount,
                min: 0,
                max: MAX_NPC_COUNT as i64,
            });
        }
        let npc_models = npc_count
            .checked_add(1)
            .filter(|models| *models <= MAX_TIMELINE_MODELS)
            .ok_or(AppError::NpcCountTooLarge)?;
        Ok(SearchConfig {
            seed,
            range_start,
            range_end,
            target,
            npc_models,
            fps: self.fps_value(),
        })
    }

    /// Target Frameを終点にして、そこから最大10,000F手前を始点にする。
    pub fn set_range_from_target(&mut self) {
        let Ok(target) = self.target.trim().parse::<i64>() else {
            return;
        };
        if !(0..=MAX_FRAME).contains(&target) {
            return;
        }
        let Ok(distance) = self.range_before_target.trim().parse::<i64>() else {
            return;
        };
        if !(0..=MAX_FRAME).contains(&distance) {
            return;
        }
        self.range_start = target.saturating_sub(distance).max(0).to_string();
        self.range_end = target.to_string();
    }

    /// 現在のオフセットから、実際の到達Frameと設定TargetFrameのTimeline差分を
    /// 補正した新しいオフセットを求める。最速閉じは差分を加算し、通常待機は
    /// B後の時間差を逆向きに補正するため減算する。
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
        let corrected = if self.fastest_close {
            current_offset.saturating_add(delta_frames)
        } else {
            current_offset.saturating_sub(delta_frames)
        };
        // オフセットは0F未満にできないため、逆方向の補正が必要でも
        // ダイアログを無反応にせず、適用可能な下限へ丸めて返す。
        Ok(corrected.max(0))
    }

    /// 使用した開始○から本番開始位置を作り、Targetと実測Frameを同じ
    /// Timeline上で比較して表示Frame差分を返す。到達可否はプレビュー側で
    /// Target／実測それぞれに確認する。
    #[cfg(test)]
    fn timeline_drift_from_circle(
        &self,
        config: &SearchConfig,
        used_circle: i64,
        actual: i64,
    ) -> Result<i64, AppError> {
        let plan = self.correction_plan(config, used_circle)?;
        self.timeline_drift_from_plan(config, &plan, actual)
    }

    fn correction_plan(
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

    fn timeline_drift_from_plan(
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

    fn correction_preview(
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
        let suggested_offset = if self.fastest_close {
            current_offset.saturating_add(drift)
        } else {
            current_offset.saturating_sub(drift)
        };
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

    fn ensure_correction_cache_readable(
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

    // --- Observation lifecycle and blink scheduling ----------------------

    pub fn apply_observation_cache(&mut self, cache: ObservationCache) {
        self.target_dirty = false;
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.observation_cache = Some(cache);
        self.clear_guidance_reachability();
        self.production = None;
        self.candidates.clear();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.timer_start = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
        self.clear_article_timer();
    }

    /// 計算中でも観測を先に開始できる入口。計算完了後に観測列を照合する。
    pub fn start_observation_pending(&mut self) {
        self.target_dirty = false;
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.observation_cache = None;
        self.clear_guidance_reachability();
        self.production = None;
        self.enter_observing();
        self.observed.clear();
        self.intervals.clear();
        self.match_tolerance = self.tolerance_value();
        self.search_exhausted = false;
        self.candidates.clear();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
        self.clear_article_timer();
    }

    pub fn cancel_observation(&mut self) {
        self.target_dirty = false;
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.observation_cache = None;
        self.enter_idle();
        self.clear_guidance_reachability();
        self.production = None;
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.observed.clear();
        self.intervals.clear();
        self.search_exhausted = false;
        self.match_tolerance = self.tolerance_value();
        self.candidates.clear();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
        self.clear_article_timer();
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn generate(&mut self) {
        let Ok(seed) = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16) else {
            return;
        };
        let Ok(a) = self.range_start.trim().parse::<i64>() else {
            return;
        };
        let Ok(b) = self.range_end.trim().parse::<i64>() else {
            return;
        };
        let Ok(target) = self.target.trim().parse::<i64>() else {
            return;
        };
        if a < 0 || b < a || target < 0 || b > MAX_FRAME || target > MAX_FRAME {
            return;
        }
        // 観測段階はNPCなし。NPC数は孵化演出終了後の本番計算でだけ使う。
        match crate::domain::rng::generate_observation_cache(seed, a..=b, target, self.fps_value())
        {
            Ok(cache) => self.observation_cache = Some(cache),
            Err(_) => {
                return;
            }
        }
        self.clear_guidance_reachability();
        self.production = None;
        self.candidates.clear();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.enter_idle();
        self.timer_start = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn start_observation(&mut self) {
        if self.observation_cache.is_none() {
            return;
        }
        self.enter_observing();
        self.clear_guidance_reachability();
        self.production = None;
        self.observed.clear();
        self.intervals.clear();
        self.candidates.clear();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
    }

    pub fn observe(&mut self) {
        if self.mode != Mode::Observing {
            return;
        }
        if self.observed.len() >= MAX_BLINK_OBSERVATIONS {
            return;
        }
        let now = Instant::now();
        self.observed.push(now);
        if let Some(previous) = self.observed.iter().rev().nth(1) {
            // 観測間隔はFieldTimelineと同じ表示Frame単位で保持する。
            // Timeline内部の1/30秒tick（=2F）への変換は検索側で行う。
            self.intervals.push(
                seconds_to_game_frames(
                    now.duration_since(*previous).as_secs_f64(),
                    self.fps_value(),
                )
                .round() as i64,
            );
        }

        // 最初のSpaceは基準点のみ。最初の瞬きまでの待ち時間は検索に含めない。
        if self.intervals.is_empty() {
            return;
        }
        if self.intervals.len() < MIN_BLINK_INTERVALS {
            self.candidates.clear();
            self.search_exhausted = false;
            return;
        }
        if self.observation_cache.is_none() {
            self.candidates.clear();
            self.search_exhausted = false;
            return;
        }

        self.rematch_candidates();
        self.selected = 0;
        self.refresh_production();
        match self.candidates.len() {
            0 => {}
            1 => {
                self.enter_ready();
            }
            _ => {}
        }
        self.schedule_blink();
    }

    /// バックグラウンド計算が終わった後、すでに入力済みの瞬き列を再照合する。
    pub fn rematch_observations(&mut self) {
        if self.mode != Mode::Observing {
            return;
        }
        if self.intervals.len() < MIN_BLINK_INTERVALS {
            self.candidates.clear();
            self.search_exhausted = false;
            return;
        }
        if self.observation_cache.is_none() {
            self.candidates.clear();
            self.search_exhausted = false;
            return;
        }
        self.rematch_candidates();
        self.selected = 0;
        self.refresh_production();
        if self.candidates.len() == 1 {
            self.enter_ready();
        }
        self.schedule_blink();
    }

    fn rematch_candidates(&mut self) {
        let mut tolerance = self
            .match_tolerance
            .max(self.tolerance_value())
            .min(MAX_AUTO_TOLERANCE);
        loop {
            let matches = self.find_matches(tolerance);
            if !matches.is_empty() || tolerance >= MAX_AUTO_TOLERANCE {
                self.candidates = matches
                    .into_iter()
                    .map(|matched| self.make_candidate(matched))
                    .collect();
                self.match_tolerance = tolerance;
                self.search_exhausted = self.candidates.is_empty();
                return;
            }
            tolerance += 1;
        }
    }

    fn find_matches(&self, tolerance: i64) -> Vec<ObservationMatch> {
        let Some(cache) = self.observation_cache.as_ref() else {
            return Vec::new();
        };
        match cache.find_matches(&self.intervals, tolerance) {
            Ok(matches) => matches,
            Err(error) => {
                self.defer_cache_io_error(error);
                Vec::new()
            }
        }
    }

    fn schedule_blink(&mut self) {
        // 候補が複数ある段階では、選択中の先頭候補を現在地として扱わない。
        // 一意に確定するまでタイマー・beep・青い現在行をすべて停止する。
        if self.candidates.len() != 1 {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        }
        // Shiftを押した実時刻が同期の基準である。最初のShiftから動かしている
        // 実測時計の経過分をそのまま使い、最後のShiftから先だけを乱数で予測する。
        // 過去の観測区間まで予測値で置き換えると、照合許容差が区間ごとに累積し、
        // 候補自体は正しくても最初のbeepが実機からずれる。
        let Some(first) = self.observed.first().copied() else {
            self.next_blink = None;
            return;
        };
        let last = self.observed.last().copied().unwrap_or(first);
        let observed_elapsed = last.saturating_duration_since(first);
        self.schedule_blink_from(first + observed_elapsed);
    }

    fn next_blink_seconds_at(&self, position: i64, fallback_ticks: i64) -> Option<f64> {
        match self
            .observation_cache
            .as_ref()
            .map(|cache| cache.interval_seconds_at(position))
        {
            Some(Ok(Some(seconds))) => Some(seconds),
            Some(Err(error)) if error.is_cache_io() => {
                self.defer_cache_io_error(error);
                None
            }
            _ => Some(blink_ticks_to_seconds(fallback_ticks, self.fps_value())),
        }
    }

    fn schedule_blink_from(&mut self, anchor: Instant) {
        let Some(candidate) = self.candidates.get(self.selected).copied() else {
            self.next_blink = None;
            return;
        };
        let Some(interval_ticks) = candidate.next_blink_ticks else {
            self.next_blink = None;
            return;
        };
        let Some(interval_seconds) =
            self.next_blink_seconds_at(candidate.sfmt_position, interval_ticks)
        else {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        };
        let deadline = anchor + Duration::from_secs_f64(interval_seconds);
        self.next_blink = Some(self.shift_deadline_by_adjust(deadline));
        self.next_blink_index = Some(candidate.sfmt_position);
    }

    /// UIの「補正」は実機との位相を合わせる実時間Frame補正であり、
    /// SFMT消費位置を変更しない。
    fn shift_deadline_by_adjust(&self, deadline: Instant) -> Instant {
        let shift = Duration::from_secs_f64(game_frames_to_seconds(
            self.adjust.unsigned_abs().min(MAX_FRAME as u64) as f64,
            self.fps_value(),
        ));
        // +補正は「音が遅いので速める」、-補正は「音が早いので遅くする」。
        if self.adjust.is_positive() {
            deadline.checked_sub(shift).unwrap_or_else(Instant::now)
        } else {
            deadline.checked_add(shift).unwrap_or(deadline)
        }
    }

    /// 予測beepを次の瞬きへ進める。実機の瞬きを入力した場合はobserveが再同期する。
    pub fn advance_predicted_blink(&mut self) {
        let Some(_candidate) = self.candidates.get(self.selected).copied() else {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        };
        // repaintは最大50ms間隔なので、beep処理が少し遅れても次の予定時刻を
        // now()基準にしない。now()を使うと、遅延が瞬きごとに累積してしまう。
        let previous_deadline = self.next_blink.unwrap_or_else(Instant::now);
        let Some(current_index) = self.next_blink_index else {
            self.schedule_blink();
            return;
        };
        // `current_index`の乱数値は、いま鳴らした瞬きまでの区間ですでに使用済み。
        // 次回は必ず1つ進めた位置の乱数値を使う。
        let next_index = current_index.saturating_add(1);
        let interval_ticks = match self
            .observation_cache
            .as_ref()
            .map(|cache| cache.interval_ticks_at(next_index))
        {
            Some(Ok(Some(ticks))) => ticks,
            Some(Err(error)) if error.is_cache_io() => {
                self.defer_cache_io_error(error);
                self.next_blink = None;
                self.next_blink_index = None;
                return;
            }
            _ => {
                self.next_blink = None;
                self.next_blink_index = None;
                return;
            }
        };
        let Some(interval_seconds) = self.next_blink_seconds_at(next_index, interval_ticks) else {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        };
        self.next_blink = Some(previous_deadline + Duration::from_secs_f64(interval_seconds));
        self.next_blink_index = Some(next_index);
        // 現在位置が進んだ場合も本番Timelineは更新するが、絶対Frameごとの
        // 到達判定結果は変わらないため、次の○の先読みを消さずに保持する。
        self.refresh_production_preserving_guidance();
    }

    /// 現在選択中の候補について、次に消費されるSFMT位置を返す。
    /// 予測瞬きが発生した後は、候補の初期位置ではなく次の位置へ進む。
    pub fn timeline_display_position(&self) -> Option<i64> {
        if self.candidates.len() != 1 {
            return None;
        }
        let candidate = self.candidates.get(self.selected)?;
        let next_position = self.next_blink_index.unwrap_or(candidate.sfmt_position);
        Some(next_position)
    }

    pub fn adjust_by(&mut self, delta: i64) {
        self.adjust = self.adjust.saturating_add(delta);
        // 補正はbeepの位相だけを動かす。SFMT位置、到達可否、ロトム判定は
        // 変えないため、本番タイムラインを再計算しない。
        let shift = Duration::from_secs_f64(game_frames_to_seconds(
            delta.unsigned_abs().min(MAX_FRAME as u64) as f64,
            self.fps_value(),
        ));
        if let Some(deadline) = self.next_blink.as_mut() {
            if delta.is_positive() {
                *deadline = deadline.checked_sub(shift).unwrap_or_else(Instant::now);
            } else {
                *deadline = deadline.checked_add(shift).unwrap_or(*deadline);
            }
        }
    }

    pub fn set_adjust(&mut self, value: i64) {
        self.adjust_by(value.saturating_sub(self.adjust));
    }

    // --- Production timer and encounter transition -----------------------

    /// 記事方式のエンカウント操作へ移行する。互換タイマーは呼び出さない。
    pub fn begin_encounter(&mut self) {
        if !self.can_start_timer() {
            return;
        }
        if self.encounter_offset_error().is_some() {
            return;
        }
        if self.candidates.get(self.selected).is_none() {
            return;
        }
        let Some(config) = self.search_config().ok() else {
            return;
        };
        if config
            .target
            .saturating_sub(self.selected_candidate_position().unwrap_or(config.target))
            > MAX_PRODUCTION_LOOKAHEAD
        {
            return;
        }
        let cache_matches_seed = self
            .observation_cache
            .as_ref()
            .is_some_and(|cache| cache.seed == config.seed);
        if !cache_matches_seed {
            return;
        }
        self.initialize_observed_rotom_talk();
        if self.production_preparation_pending {
            self.encounter_start_requested = true;
            return;
        }
        let Some(production) = self.production else {
            self.mark_production_preparation_pending();
            self.encounter_start_requested = true;
            return;
        };
        if production.target_tick.is_none() {
            return;
        }
        let Some(_plan) = self.article_plan_for_selected() else {
            return;
        };

        self.production = Some(production);
        self.encounter_start_requested = false;
        // 補正はSet後の表示値ではなく、本番へ移行した時点の値を基準にする。
        self.correction_source_offset = Some(self.encounter_offset_value());
        // Readyを押した時点から区間を最初から数え直さず、Shift観測と補正から
        // すでに予約済みの次回瞬きまでの「残り時間」を引き継ぐ。
        if self.fastest_close {
            // 最速閉じは○までの案内だけで完了する。○表示後にアプリ側で
            // Enterを受けたりTargetタイマーを開始したりしない。
            return;
        }

        // 追加待機方式では、現在表示が○の時点でEnterと実機Bを同時に押す。
        // B後オフセット中はTimelineを進めない。オフセットとTargetまでの本番待機を
        // 別々に表示し、区切りでbeepを鳴らせるよう二段階で計時する。
        let target_seconds = production
            .target_tick
            .map(|ticks| self.game_ticks_to_seconds(ticks))
            .unwrap_or(0.0);
        self.correction_source_frame = self.timeline_display_position();
        self.correction_source_index = 0;
        self.correction_target_frame = Some(production.target);
        let offset_seconds = self.encounter_offset_seconds();
        self.next_blink = None;
        if self.use_offset {
            self.article_next_seconds = Some(target_seconds);
            self.start_article_timer(ArticleTimerStage::Offset, offset_seconds);
        } else {
            self.article_next_seconds = None;
            self.start_article_timer(ArticleTimerStage::ToTarget, target_seconds);
        }
    }

    /// 最速閉じの○を実際に使用した時点で、瞬き予測を停止して
    /// ずれ検証へ渡せる状態にする。オフセットの有効／無効には依存しない。
    pub fn finish_fastest_close_for_correction(&mut self) -> bool {
        if !self.can_finish_fastest_close_for_correction() {
            return false;
        }
        let Some(current) = self.timeline_display_position() else {
            return false;
        };
        self.initialize_observed_rotom_talk();
        // 最速閉じでは記録済みの○だけをB押下候補として残す。現在位置が×でも、
        // 既に記録した○を補正元へ使い、到達不能な現在位置は候補へ追加しない。
        let frames = self.correction_start_candidates();
        let source_index = frames
            .iter()
            .position(|frame| *frame == current)
            .unwrap_or(0);
        let Some(source_frame) = frames.get(source_index).copied() else {
            return false;
        };
        self.correction_source_frame = Some(source_frame);
        self.correction_source_index = source_index;
        self.correction_target_frame = self.search_config().ok().map(|config| config.target);
        self.correction_source_offset = Some(self.encounter_offset_value());
        self.next_blink = None;
        self.next_blink_index = Some(source_frame);
        self.timer_start = None;
        self.clear_article_timer();
        self.enter_finished();
        true
    }

    pub fn remaining(&self) -> f64 {
        self.remaining_at(Instant::now())
    }

    fn remaining_at(&self, now: Instant) -> f64 {
        self.timer_start
            .map(|start| {
                (self.timer_seconds - now.saturating_duration_since(start).as_secs_f64()).max(0.0)
            })
            .unwrap_or(0.0)
    }

    pub fn article_remaining(&self) -> f64 {
        self.article_remaining_at(Instant::now())
    }

    fn article_remaining_at(&self, now: Instant) -> f64 {
        self.article_timer_start
            .map(|start| {
                (self.article_timer_seconds - now.saturating_duration_since(start).as_secs_f64())
                    .max(0.0)
            })
            .unwrap_or(0.0)
    }

    /// 記事方式のカウントダウンを進め、段階が切り替わったときにその段階を返す。
    pub fn tick_article_timer(&mut self) -> Option<ArticleTimerStage> {
        self.tick_article_timer_at(Instant::now())
    }

    fn tick_article_timer_at(&mut self, now: Instant) -> Option<ArticleTimerStage> {
        if self.mode != Mode::Countdown || self.article_remaining_at(now) > 0.0 {
            return None;
        }
        let stage = self.article_timer_stage?;
        match stage {
            ArticleTimerStage::Offset => {
                let target_seconds = self.article_next_seconds.take().unwrap_or(0.0);
                let next_start = self
                    .article_timer_start
                    .map(|start| start + Duration::from_secs_f64(self.article_timer_seconds))
                    .unwrap_or(now);
                self.article_timer_stage = Some(ArticleTimerStage::ToTarget);
                self.article_timer_seconds = target_seconds;
                self.article_timer_start = Some(next_start);
            }
            ArticleTimerStage::ToTarget => {
                self.enter_finished();
                self.clear_article_timer();
            }
        }
        Some(stage)
    }

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

    fn pre_rotom_consumption_value(&self) -> i64 {
        self.pre_rotom_consumption
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|value| (0..=MAX_FRAME).contains(value))
            .filter(|_| self.consider_pre_rotom_consumption)
            .unwrap_or(0)
    }

    /// 候補の現在SFMT位置を返す。UIの実時間補正は位置へ加算しない。
    fn candidate_position(&self, candidate_pos: usize) -> Option<i64> {
        let candidate = self.candidates.get(candidate_pos)?;
        let position = if candidate_pos == self.selected {
            self.next_blink_index.unwrap_or(candidate.sfmt_position)
        } else {
            candidate.sfmt_position
        };
        Some(position)
    }

    fn selected_candidate_position(&self) -> Option<i64> {
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
    fn rotom_correction_from(&self, used_circle: i64, _actual: i64) -> Option<RotomCorrection> {
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

    fn tolerance_value(&self) -> i64 {
        self.tolerance.trim().parse::<i64>().unwrap_or(5).max(0)
    }

    fn encounter_offset_value(&self) -> i64 {
        if self.use_offset {
            self.encounter_offset.trim().parse::<i64>().unwrap_or(0)
        } else {
            0
        }
    }

    fn correction_source_offset_value(&self) -> i64 {
        self.correction_source_offset
            .unwrap_or_else(|| self.encounter_offset_value())
    }

    /// 本番へ移行した時点の予測を、補正画面で選択する初期値として保持する。
    /// 補正中に選択を変更しても、右上の到達可否計算は設定値を使い続ける。
    fn initialize_observed_rotom_talk(&mut self) {
        if self.observed_rotom_talk.is_none() && self.consider_rotom_talk {
            self.observed_rotom_talk = self.predicted_rotom_talk();
        }
    }

    fn plan_for_encounter(
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

    fn npc_initial_load_value(&self) -> i64 {
        self.npc_initial_load
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|value| (0..=50).contains(value))
            .filter(|_| self.consider_npc_initial_load)
            .unwrap_or(0)
    }

    fn encounter_offset_ticks(&self) -> Option<i64> {
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
    fn encounter_offset_seconds(&self) -> f64 {
        game_frames_to_seconds(
            self.encounter_offset_value().max(0) as f64,
            self.fps_value(),
        )
    }

    /// 本番タイムラインの1ステップは、外部ツール上では実機2F相当。
    /// `target_tick`はSFMT消費位置ではなく、時間経過ステップ数なので、
    /// TargetFrameの差分と同じように扱わない。
    fn game_ticks_to_seconds(&self, ticks: i64) -> f64 {
        blink_ticks_to_seconds(ticks, self.fps_value())
    }

    fn start_article_timer(&mut self, stage: ArticleTimerStage, seconds: f64) {
        self.article_timer_stage = Some(stage);
        self.article_timer_seconds = seconds.max(0.0);
        self.article_timer_start = Some(Instant::now());
        self.enter_countdown();
    }

    fn clear_article_timer(&mut self) {
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

    /// Target確定後に必要なSFMT範囲を計算し、本番Timelineを準備する要求を作る。
    pub(crate) fn production_preparation_request(
        &self,
    ) -> Option<(ProductionPreparationRequest, ObservationCache)> {
        if self.target_dirty
            || self.encounter_offset_error().is_some()
            || self.candidates.get(self.selected).is_none()
        {
            return None;
        }
        let config = self.search_config().ok()?;
        let observed_position = self.selected_candidate_position()?;
        if config.target < observed_position
            || config.target.saturating_sub(observed_position) > MAX_PRODUCTION_LOOKAHEAD
        {
            return None;
        }
        let query = ProductionQuery {
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            observed_rotom_talk: self.observed_rotom_talk,
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks: self
                .fastest_close
                .then(|| self.encounter_offset_ticks())
                .flatten(),
        };
        if self.fastest_close && query.fastest_ticks.is_none() {
            return None;
        }
        let plan = self.plan_for_encounter(
            config.seed,
            observed_position,
            query.offset,
            query.consider_rotom,
            query.rotom_threshold,
            query.observed_rotom_talk,
        )?;
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == config.seed)?
            .clone();
        Some((
            ProductionPreparationRequest {
                session_generation: self.guidance.session_generation(),
                selected_candidate: self.selected,
                observed_position,
                encounter_start: plan.encounter_start,
                query,
            },
            cache,
        ))
    }

    /// ワーカー上でSFMTキャッシュを必要位置まで延長し、本番Timelineを計算する。
    pub(crate) fn calculate_production_preparation(
        request: ProductionPreparationRequest,
        cache: ObservationCache,
    ) -> Result<PreparedProduction, AppError> {
        let query = request.query;
        if cache.seed != query.seed {
            return Err(AppError::Cache(CacheError::SeedMismatch));
        }
        let base_start = request.encounter_start;
        let mut start = base_start;
        if let Some(ticks) = query.fastest_ticks {
            let advance_values = ticks
                .max(0)
                .saturating_mul(query.models as i64)
                .saturating_add(query.models as i64 + 1);
            let required_end = base_start.saturating_add(advance_values).min(MAX_FRAME);
            cache.ensure_through(required_end, None)?;
            let count = usize::try_from(required_end.saturating_sub(base_start) + 1)
                .map_err(|_| AppError::ArithmeticOverflow(ArithmeticOperation::TimelineLength))?;
            let values = cache.read_values(base_start, count)?;
            start =
                advance_timeline_from_values(&values, base_start, base_start, query.models, ticks)
                    .ok_or(AppError::InvalidState(StateIssue::TimelineUnavailable))?;
        }

        let timeline_values = PRODUCTION_TIMELINE_CACHE_FRAMES
            .saturating_add(query.models as i64)
            .saturating_add(1);
        // Timeline用の固定窓を要求する。ObservationCache側が要求位置の先に
        // 1チャンクを維持し、末尾1チャンクへ入った時点で次を追記する。
        let required_end = start
            .saturating_add(timeline_values)
            .saturating_sub(1)
            .min(MAX_FRAME);
        cache.ensure_through(required_end, None)?;

        let available = cache
            .available_end()
            .saturating_sub(start)
            .saturating_add(1);
        let count = usize::try_from(available.min(timeline_values).max(0))
            .map_err(|_| AppError::ArithmeticOverflow(ArithmeticOperation::TimelineLength))?;
        let values = cache.read_values(start, count)?;
        let mut timeline = ProductionTimelineCache::new_from_values(
            query.seed,
            start,
            query.models,
            PRODUCTION_TIMELINE_CACHE_FRAMES,
            values,
        )
        .ok_or(AppError::InvalidState(StateIssue::TimelineUnavailable))?;
        timeline.extend_until(query.target);
        let target_tick = timeline
            .target_from(start, query.target)
            .and_then(|timing| timing.exact_tick);
        Ok(PreparedProduction {
            cache: timeline,
            production: ProductionTimeline {
                target: query.target,
                target_tick,
            },
        })
    }

    /// 入力条件がワーカー要求と同じ場合だけ、計算済みTimelineを状態へ反映する。
    pub(crate) fn apply_production_preparation(
        &mut self,
        request: &ProductionPreparationRequest,
        prepared: PreparedProduction,
    ) -> bool {
        let matches = self
            .production_preparation_request()
            .is_some_and(|(current, _)| current == *request);
        if !matches {
            return false;
        }
        self.production_cache = Some(prepared.cache);
        self.production = Some(prepared.production);
        for (index, candidate) in self.candidates.iter_mut().enumerate() {
            candidate.target_reachable =
                index == self.selected && prepared.production.target_tick.is_some();
        }
        self.production_preparation_pending = false;
        if std::mem::take(&mut self.encounter_start_requested) {
            self.begin_encounter();
        }
        true
    }

    pub(crate) fn mark_production_preparation_pending(&mut self) {
        self.clear_guidance_reachability();
        self.production = None;
        self.production_preparation_pending = true;
        self.encounter_start_requested = false;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    pub(crate) fn mark_production_preparation_pending_preserving_guidance(&mut self) {
        self.production = None;
        self.production_preparation_pending = true;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    pub(crate) fn finish_production_preparation_without_result(&mut self) {
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.production = None;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    /// 通常待機でTargetを正確に踏めない場合でも、TimelineがTargetを通過する
    /// ステップを待機時間の目安として返す。○×とは独立した表示用の値である。
    fn normal_target_wait_ticks(&self, position: i64) -> Result<Option<i64>, AppError> {
        let Ok(config) = self.search_config() else {
            return Ok(None);
        };
        if config.target.saturating_sub(position) > MAX_PRODUCTION_LOOKAHEAD {
            return Ok(None);
        }
        let Some(start) = self.plan_for_encounter(
            config.seed,
            position,
            self.encounter_offset_value(),
            self.consider_rotom_talk,
            self.rotom_threshold_value(),
            self.observed_rotom_talk,
        ) else {
            return Ok(None);
        };
        let start = start.encounter_start;
        let cached_timing = self
            .production_cache
            .as_ref()
            .filter(|cache| cache.seed == config.seed && cache.models == config.npc_models)
            .and_then(|cache| cache.target_from(start, config.target));
        if let Some(timing) = cached_timing {
            return Ok(Some(timing.timeline_tick));
        }
        match self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == config.seed)
        {
            Some(cache) => {
                Ok(
                    timeline_target_from_cache(cache, start, config.target, config.npc_models)?
                        .map(|timing| timing.timeline_tick),
                )
            }
            None => Ok(None),
        }
    }

    fn make_candidate(&self, matched: ObservationMatch) -> TimelineCandidate {
        TimelineCandidate {
            start_position: matched.start_position,
            sfmt_position: matched.sfmt_position,
            next_blink_ticks: Some(matched.next_blink_ticks),
            // 選択候補の本番到達可否は、候補確定後にWorkerで計算して反映する。
            target_reachable: false,
        }
    }

    fn refresh_production(&mut self) {
        if self.encounter_offset_error().is_some() {
            self.production_preparation_pending = false;
            for candidate in &mut self.candidates {
                candidate.target_reachable = false;
            }
            self.production = None;
            return;
        }
        self.mark_production_preparation_pending();
    }

    /// 現在位置だけが進んだ場合は、本番Timelineを更新しつつ、絶対位置ごとの
    /// 到達判定キャッシュは維持する。設定変更時の通常更新とは分ける。
    fn refresh_production_preserving_guidance(&mut self) {
        if self.encounter_offset_error().is_some() {
            self.production_preparation_pending = false;
            self.encounter_start_requested = false;
            for candidate in &mut self.candidates {
                candidate.target_reachable = false;
            }
            self.production = None;
            return;
        }
        self.mark_production_preparation_pending_preserving_guidance();
    }

    // --- Candidate reachability and guidance ------------------------------

    /// 候補ごとの本番到達判定をワーカースレッドへ渡すスナップショット。
    pub fn candidate_reachability_request(&self) -> Option<CandidateReachabilityRequest> {
        if self.candidates.len() <= 1 || self.encounter_offset_error().is_some() {
            return None;
        }
        let config = self.search_config().ok()?;
        let fastest_ticks = self
            .fastest_close
            .then(|| self.encounter_offset_ticks())
            .flatten();
        if self.fastest_close && fastest_ticks.is_none() {
            return None;
        }
        Some(CandidateReachabilityRequest {
            session_generation: self.guidance.session_generation(),
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            // 実測のお喋り有無は補正／本番計算専用。右上の○×は設定値で判定する。
            observed_rotom_talk: None,
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks,
            candidate_positions: self
                .candidates
                .iter()
                .enumerate()
                .filter_map(|(index, _)| self.candidate_position(index).map(|frame| (index, frame)))
                .collect(),
        })
    }

    /// 候補ごとの本番Timeline判定。重い処理なのでUiAppからバックグラウンド
    /// で呼び出し、完了後に同じ入力スナップショットへだけ反映する。
    #[cfg(test)]
    pub fn calculate_candidate_reachability(
        request: CandidateReachabilityRequest,
    ) -> Vec<(usize, bool)> {
        request
            .candidate_positions
            .into_par_iter()
            .map(|(candidate_pos, current)| {
                if request.target.saturating_sub(current) > MAX_PRODUCTION_LOOKAHEAD {
                    return (candidate_pos, false);
                }
                let reachable = ProductionQuery {
                    seed: request.seed,
                    target: request.target,
                    models: request.models,
                    offset: request.offset,
                    consider_rotom: request.consider_rotom,
                    rotom_threshold: request.rotom_threshold,
                    observed_rotom_talk: request.observed_rotom_talk,
                    pre_rotom_consumption: request.pre_rotom_consumption,
                    npc_initial_load: request.npc_initial_load,
                    fastest_ticks: request.fastest_ticks,
                }
                .target_reachable(current);
                (candidate_pos, reachable)
            })
            .collect()
    }

    /// 候補複数時の到達判定を、観測時に作成したSFMTキャッシュから計算する。
    ///
    /// 候補ごとに`ProductionQuery`を呼ぶと、それぞれがSeedから開始Frameまで
    /// SFMTを再生する。候補範囲を一度だけ位置指定で読み、同じ列を共有する。
    pub fn calculate_candidate_reachability_from_cache(
        request: CandidateReachabilityRequest,
        cache: &ObservationCache,
    ) -> Result<Vec<(usize, bool)>, AppError> {
        let false_results = || {
            request
                .candidate_positions
                .iter()
                .map(|(candidate, _)| (*candidate, false))
                .collect::<Vec<_>>()
        };
        if cache.seed != request.seed {
            return Err(AppError::Cache(CacheError::SeedMismatch));
        }
        if request.target.saturating_sub(
            request
                .candidate_positions
                .iter()
                .map(|(_, frame)| *frame)
                .min()
                .unwrap_or(request.target),
        ) > MAX_PRODUCTION_LOOKAHEAD
        {
            return Ok(false_results());
        }
        let Some(min_observed) = request
            .candidate_positions
            .iter()
            .map(|(_, frame)| *frame)
            .min()
        else {
            return Ok(Vec::new());
        };
        let max_observed = request
            .candidate_positions
            .iter()
            .map(|(_, frame)| *frame)
            .max()
            .unwrap_or(min_observed);
        let Some(b_start) = min_observed.checked_add(request.pre_rotom_consumption) else {
            return Ok(false_results());
        };
        let Some(b_count) = max_observed
            .checked_sub(min_observed)
            .and_then(|span| span.checked_add(1))
            .and_then(|count| usize::try_from(count).ok())
        else {
            return Ok(false_results());
        };
        let b_values = cache.read_values(b_start, b_count)?;
        let mut planned = Vec::with_capacity(request.candidate_positions.len());
        for &(candidate, observed) in &request.candidate_positions {
            let Some(value) = observed
                .checked_sub(min_observed)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| b_values.get(offset).copied())
            else {
                return Ok(false_results());
            };
            let Some(plan) = crate::domain::rng::encounter_plan_with_pre_rotom_value(
                observed,
                request.offset,
                request.consider_rotom,
                request.rotom_threshold,
                request.observed_rotom_talk,
                request.pre_rotom_consumption,
                request.npc_initial_load,
                value,
            ) else {
                return Ok(false_results());
            };
            planned.push((candidate, plan.encounter_start));
        }
        if let Some(ticks) = request.fastest_ticks {
            let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
                return Ok(Vec::new());
            };
            let max_start = planned
                .iter()
                .map(|(_, start)| *start)
                .max()
                .unwrap_or(min_start);
            let Some(count) = max_start
                .checked_sub(min_start)
                .and_then(|span| {
                    span.checked_add(ticks.max(0).saturating_mul(request.models as i64))
                })
                .and_then(|span| span.checked_add(request.models as i64 + 1))
                .and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false_results());
            };
            let values = cache.read_values(min_start, count)?;
            for (_, start) in &mut planned {
                let Some(advanced) =
                    advance_timeline_from_values(&values, min_start, *start, request.models, ticks)
                else {
                    return Ok(false_results());
                };
                *start = advanced;
            }
        }
        let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
            return Ok(Vec::new());
        };
        let max_start = planned
            .iter()
            .map(|(_, start)| *start)
            .max()
            .unwrap_or(min_start);
        let by_start: HashMap<i64, bool> = target_reachability_range_from_cache(
            cache,
            min_start..=max_start,
            request.target,
            request.models,
            None,
        )?
        .into_iter()
        .collect();
        Ok(planned
            .into_iter()
            .map(|(candidate, start)| (candidate, by_start.get(&start).copied().unwrap_or(false)))
            .collect())
    }

    pub fn apply_candidate_reachability(
        &mut self,
        request: &CandidateReachabilityRequest,
        results: &[(usize, bool)],
    ) {
        if !self
            .candidate_reachability_request()
            .is_some_and(|current| current == *request)
        {
            return;
        }
        for &(candidate_pos, reachable) in results {
            if let Some(candidate) = self.candidates.get_mut(candidate_pos) {
                candidate.target_reachable = reachable;
            }
        }
    }

    pub fn recalculate_production(&mut self) {
        self.clear_guidance_reachability();
        self.fast_close_circle_frames.clear();
        self.production = None;
        self.production_preparation_pending = !self.candidates.is_empty();
        self.encounter_start_requested = false;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    /// 実測ロトム結果だけを更新したときの本番経路更新。右上の○×は
    /// 設定確率だけで判定するため、既に計算済みの案内結果を消さない。
    pub fn recalculate_production_preserving_guidance(&mut self) {
        self.mark_production_preparation_pending_preserving_guidance();
    }

    /// 表の指定位置からTarget Frameへ到達できるかを遅延計算する。
    /// 到達可否は瞬き検索の結果ではなく、本番NPCタイムラインの結果で決まる。
    #[cfg(test)]
    pub fn table_target_reachable(&mut self, frame: i64) -> Option<bool> {
        if let Some(reachable) = self.guidance.reachability(frame) {
            return Some(reachable);
        }
        let config = self.search_config().ok()?;
        if config.target.saturating_sub(frame) > MAX_PRODUCTION_LOOKAHEAD {
            self.record_guidance_reachability(frame, false);
            return Some(false);
        }
        let plan = self.plan_for_encounter(
            config.seed,
            frame,
            self.encounter_offset_value(),
            self.consider_rotom_talk,
            self.rotom_threshold_value(),
            None,
        )?;
        // 観測候補が1つに決まった後は、その候補上の未来位置に同じ本番経路を再利用する。
        // 瞬き表示ごとのModelStatus再初期化を避け、経路外の位置だけ独立計算へ戻す。
        if !self.fastest_close && self.candidates.len() == 1 {
            if let Some(cache) = self.production_cache.as_ref() {
                if cache.seed == config.seed && cache.models == config.npc_models {
                    if let Some(timing) = cache.target_from(plan.encounter_start, config.target) {
                        let reachable = timing.exact_tick.is_some();
                        self.record_guidance_reachability(frame, reachable);
                        return Some(reachable);
                    }
                }
            }
        }
        let reachable = ProductionQuery {
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            observed_rotom_talk: None,
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks: self
                .fastest_close
                .then(|| self.encounter_offset_ticks())
                .flatten(),
        }
        .target_exactly_reached(frame)
        .unwrap_or(false);
        self.record_guidance_reachability(frame, reachable);
        Some(reachable)
    }

    /// 次の右上案内チャンクを作るための入力を返す。ここでは範囲と現在の
    /// キャッシュ終端だけを確認し、SFMT計算は行わない。
    pub fn guidance_search_request(&self) -> Option<GuidanceSearchRequest> {
        // Target欄の編集中は、1文字ごとの暫定値で案内Workerを起動しない。
        // 確定時にUIが案内結果を破棄してから、新しい設定で再検索する。
        if self.target_dirty {
            return None;
        }
        let plan = self.guidance_scan_plan()?;
        if plan.target_too_far {
            return None;
        }
        let observed_start = plan.known_end.saturating_add(1).max(plan.search_start);
        if observed_start > plan.search_end {
            return None;
        }
        let observed_end = observed_start
            .saturating_add(GUIDANCE_CHUNK.saturating_sub(1))
            .min(plan.search_end);
        let config = plan.config;
        Some(GuidanceSearchRequest {
            session_generation: self.guidance.session_generation(),
            observed_start,
            observed_end,
            range_start: config.range_start,
            range_end: config.range_end,
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            // 右上の○×案内は実測ドロップダウンから独立させる。
            observed_rotom_talk: None,
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks: plan.fastest_ticks,
        })
    }

    /// バックグラウンド計算済みの案内結果だけを状態へ反映する。
    pub fn apply_guidance_results(
        &mut self,
        request: GuidanceSearchRequest,
        results: Vec<(i64, bool)>,
    ) {
        // Workerの完了順は保証されないため、状態側でも現在の設定と同じ
        // セッションの結果だけを受け付ける。UI側の照合を経由しない呼び出し
        // でも、古いSeed・Target・オフセットの結果が案内へ混入しない。
        let Some(current_request) = self.guidance_search_request() else {
            return;
        };
        if !current_request.same_configuration(request) {
            return;
        }
        let expected_len = request
            .observed_end
            .checked_sub(request.observed_start)
            .and_then(|span| span.checked_add(1))
            .and_then(|length| usize::try_from(length).ok());
        let complete = expected_len.is_some_and(|length| {
            results.len() == length
                && results.iter().enumerate().all(|(index, (frame, _))| {
                    request.observed_start.saturating_add(index as i64) == *frame
                })
        });
        if !complete {
            return;
        }
        for (frame, reachable) in results {
            self.record_guidance_reachability(frame, reachable);
        }
        self.guidance.mark_computed_until(request.observed_end);
    }

    /// 右上の○×案内。現在位置から次に○となる孵化瞬き位置と、その位置が
    /// 続く実時間を同じSFMT行から求める。
    pub fn hatch_guidance(&mut self) -> Result<Option<HatchGuidance>, AppError> {
        let Some(cache) = self.observation_cache.clone() else {
            return Ok(None);
        };
        let Some(plan) = self.guidance_scan_plan() else {
            return Ok(None);
        };
        if plan.target_too_far {
            return Ok(Some(HatchGuidance {
                current_frame: plan.current,
                current_reachability: Some(false),
                reachable_now: false,
                blinks_until_circle: 0,
                next_circle_frame: None,
                next_circle_seconds: None,
                grace_seconds: None,
                target_wait_seconds: None,
                target_unreachable: false,
                search_out_of_range: true,
            }));
        }
        let current = plan.current;
        // 最速閉じの理論範囲外はWorkerへ渡していないが、到達不能である
        // ことが既に確定している。未計算(None)として扱うと、検索完了後も
        // 現在位置が`?`のままになるため、ここで確定×へ変換する。
        let current_reachability =
            if self.fastest_close && (current < plan.search_start || current > plan.search_end) {
                Some(false)
            } else {
                self.guidance.reachability(current)
            };
        let fastest_bounds = plan.fastest_bounds;
        let search_start = plan.search_start;
        let scan_end = plan.known_end.min(plan.search_end);
        // 理論上の候補範囲より前は最速閉じでは○にならないため、そこを
        // 計算せずに飛ばしつつ、到達可能位置の順序付き索引から次○を取る。
        let next_circle = (search_start <= scan_end)
            .then(|| self.guidance.next_reachable(search_start, scan_end))
            .flatten();
        let blinks = next_circle
            .map(|frame| {
                frame
                    .saturating_sub(current)
                    .try_into()
                    .unwrap_or(usize::MAX)
            })
            .unwrap_or_else(|| {
                let frames_checked = scan_end
                    .checked_sub(search_start)
                    .and_then(|value| value.checked_add(1))
                    .unwrap_or(0);
                search_start
                    .saturating_sub(current)
                    .saturating_add(frames_checked)
                    .try_into()
                    .unwrap_or(usize::MAX)
            });
        let reachable_now = current_reachability == Some(true);
        // 現在が○のときは現在行を数えず、その次に現れる○までの
        // 瞬き回数を表示する。現在が×のときは従来どおり、現在位置
        // から最初の○までの回数を表示する。
        let blinks_until_circle = if reachable_now {
            let next_start = current.saturating_add(1);
            if next_start <= scan_end {
                self.guidance
                    .next_reachable(next_start, scan_end)
                    .map(|frame| {
                        frame
                            .saturating_sub(current)
                            .try_into()
                            .unwrap_or(usize::MAX)
                    })
                    .unwrap_or_else(|| 0)
            } else {
                0
            }
        } else {
            blinks
        };
        if self.fastest_close && reachable_now && !self.fast_close_circle_frames.contains(&current)
        {
            self.fast_close_circle_frames.push(current);
            if self.fast_close_circle_frames.len() > 16 {
                self.fast_close_circle_frames.remove(0);
            }
        }
        let search_pending = self.guidance_search_pending();
        let next_circle_seconds = if let Some(end) = next_circle {
            if let Some(count) = end
                .checked_sub(current)
                .and_then(|value| usize::try_from(value).ok())
            {
                if count == 0 {
                    Some(0.0)
                } else if let Some(intervals) =
                    self.guidance_interval_seconds_range(&cache, current, count)?
                {
                    let full_seconds: f64 = intervals.iter().sum();
                    // 最初の区間は既に一部経過している場合がある。瞬きタイマー実行中は区間全体の
                    // 長さを現在の締切へ置き換え、整数表示を実時間の経過に合わせる。
                    intervals.first().copied().map(|first_full| {
                        let first_remaining = self
                            .next_blink
                            .map(|deadline| {
                                deadline
                                    .saturating_duration_since(Instant::now())
                                    .as_secs_f64()
                            })
                            .unwrap_or(first_full);
                        (full_seconds - first_full + first_remaining).max(0.0)
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let grace_seconds = if let Some(start) = next_circle {
            let mut total = 0.0;
            // 未来の検索が続いている場合、既知範囲の末尾まで○が続くだけ
            // では猶予秒数を確定できない。既知の×に到達した場合は、そこが
            // ○区間の正常な終端なので、その時点で確定できる。
            let mut complete = !search_pending;
            let Some(count) = scan_end
                .checked_sub(start)
                .and_then(|value| value.checked_add(1))
                .and_then(|value| usize::try_from(value).ok())
            else {
                return Ok(None);
            };
            if let Some(seconds) = self.guidance_interval_seconds_range(&cache, start, count)? {
                for (offset, seconds) in seconds.into_iter().enumerate() {
                    let frame = start.saturating_add(offset as i64);
                    match self.guidance.reachability(frame) {
                        Some(true) => total += seconds,
                        Some(false) => {
                            complete = true;
                            break;
                        }
                        None => {
                            complete = false;
                            break;
                        }
                    }
                }
            }
            (complete && total > 0.0).then_some(total)
        } else {
            None
        };
        // Targetの検索範囲外表示は、現在地からの共通上限を超えた場合
        // だけに限定する。通常待機では観測範囲の終点を越えても、現在の
        // TimelineからTargetまでの本番待機時間を計算できる。
        let search_out_of_range = !self.target_dirty && plan.target_too_far;
        // 最速閉じでは理論上の○が残っていない場合、通常待機ではTargetが
        // 現在位置より前にある場合を到達不能として表示する。検索範囲外は
        // 到達不能とは別の状態なので、そちらを優先する。
        let fastest_target_unreachable = self.fastest_close
            && next_circle.is_none()
            && !self.guidance_search_pending()
            && fastest_bounds.is_some_and(|bounds| bounds.2);
        let target_unreachable = !self.target_dirty
            && !search_out_of_range
            && (fastest_target_unreachable
                || (!self.fastest_close && plan.config.target < current));
        let target_wait_seconds = if self.fastest_close || self.production_preparation_pending {
            None
        } else {
            let target_ticks = self
                .production
                .as_ref()
                .and_then(|production| production.target_tick);
            let target_ticks = match target_ticks {
                Some(ticks) => Some(ticks),
                None => match self.normal_target_wait_ticks(current)? {
                    Some(ticks) => Some(ticks),
                    None => match next_circle {
                        Some(frame) => self.normal_target_wait_ticks(frame)?,
                        None => None,
                    },
                },
            };
            target_ticks.map(|ticks| self.game_ticks_to_seconds(ticks))
        };
        Ok(Some(HatchGuidance {
            current_frame: current,
            current_reachability,
            reachable_now,
            blinks_until_circle,
            next_circle_frame: next_circle,
            next_circle_seconds,
            grace_seconds,
            target_wait_seconds,
            target_unreachable,
            search_out_of_range,
        }))
    }

    /// 右上案内の次チャンクを継続計算する必要があるか。
    pub fn guidance_search_pending(&self) -> bool {
        if self.target_dirty {
            return false;
        }
        let Some(plan) = self.guidance_scan_plan() else {
            return false;
        };
        !plan.target_too_far && plan.known_end < plan.search_end
    }

    /// 最速閉じでTargetへ到達し得る観測位置の範囲を返す。
    ///
    /// 1ステップで消費できるSFMT値は、現在のModelStatus実装ではモデル1体
    /// あたり最大1個である。そのため、`k = オフセットF / 2` ステップ後に
    /// Targetへ到達する開始位置は `Target - k * models` より前にならない。
    /// 観測位置はロトム総消費・NPC初期読み込みの分だけさらに手前なので、
    /// そこへ100Fの安全余白を加えて検索する。戻り値は
    /// (検索下限, 検索上限, 設定範囲が理論範囲全体を覆うか) の順。
    fn fastest_close_guidance_bounds(&self, config: &SearchConfig) -> Option<(i64, i64, bool)> {
        let ticks = self.encounter_offset_ticks()?;
        let models = i64::try_from(config.npc_models).ok()?;
        let max_consumption = ticks.checked_mul(models)?;
        let rotom_max = if self.consider_rotom_talk { 2 } else { 0 };
        let rotom_min = if self.consider_rotom_talk { 1 } else { 0 };
        let pre_rotom_consumption = self.pre_rotom_consumption_value();
        let npc_initial_load = self.npc_initial_load_value();
        let theoretical_start = config
            .target
            .saturating_sub(max_consumption)
            .saturating_sub(pre_rotom_consumption)
            .saturating_sub(rotom_max)
            .saturating_sub(npc_initial_load);
        let theoretical_end = config
            .target
            .saturating_sub(pre_rotom_consumption)
            .saturating_sub(rotom_min)
            .saturating_sub(npc_initial_load);
        let effective_theoretical_start = theoretical_start.max(0);
        let effective_theoretical_end = theoretical_end.max(0);
        let search_start = theoretical_start
            .saturating_sub(FASTEST_CLOSE_SEARCH_BUFFER)
            .max(config.range_start)
            .max(0);
        let search_end = theoretical_end
            .saturating_add(FASTEST_CLOSE_SEARCH_BUFFER)
            .min(MAX_FRAME);
        let fully_covered =
            search_start <= effective_theoretical_start && search_end >= effective_theoretical_end;
        Some((search_start, search_end, fully_covered))
    }

    #[cfg(test)]
    pub fn calculate_guidance_batch(request: GuidanceSearchRequest) -> Vec<(i64, bool)> {
        // UIは表示中の現在位置から始まるチャンクだけを要求する。Targetが本番窓の上限を
        // 超えている場合は全てfalseの軽い結果を返し、遠いTargetまでSFMTを生成しない。
        if request.target.saturating_sub(request.observed_start) > MAX_PRODUCTION_LOOKAHEAD {
            return (request.observed_start..=request.observed_end)
                .map(|frame| (frame, false))
                .collect();
        }
        let query = ProductionQuery {
            seed: request.seed,
            target: request.target,
            models: request.models,
            offset: request.offset,
            consider_rotom: request.consider_rotom,
            rotom_threshold: request.rotom_threshold,
            observed_rotom_talk: request.observed_rotom_talk,
            pre_rotom_consumption: request.pre_rotom_consumption,
            npc_initial_load: request.npc_initial_load,
            fastest_ticks: request.fastest_ticks,
        };
        let mut planned = Vec::new();
        for frame in request.observed_start..=request.observed_end {
            // `target_reachability_range`のtickをB+ロトム開始位置から適用する。
            // ここは開始位置そのものを表すため、同じtickをここでも進めるとTarget比較
            // 前に全候補を二重に移動してしまうため進めない。
            if let Some(start) = query.base_encounter_start(frame) {
                planned.push((frame, start));
            }
        }
        let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
            return Vec::new();
        };
        let max_start = planned
            .iter()
            .map(|(_, start)| *start)
            .max()
            .unwrap_or(min_start);
        let by_start: HashMap<i64, bool> = target_reachability_range(
            request.seed,
            min_start..=max_start,
            request.target,
            request.models,
            request.fastest_ticks,
        )
        .into_iter()
        .collect();
        // 要求された各観測Frameに必ず1行を返す。行を落とすと表示側の計算済み終点だけが
        // 未計算位置を越え、UIに横棒が残り続ける。
        let start_by_observed: HashMap<i64, i64> = planned.into_iter().collect();
        (request.observed_start..=request.observed_end)
            .map(|observed| {
                let reachable = start_by_observed
                    .get(&observed)
                    .and_then(|start| by_start.get(start))
                    .copied()
                    .unwrap_or(false);
                (observed, reachable)
            })
            .collect()
    }

    /// 既生成のSFMTキャッシュだけを使って、右上案内の1チャンクを計算する。
    ///
    /// チャンクごとにSeedから現在位置まで再生すると、高いFrameでは同じ長い
    /// SFMT prefixを何度も再計算することになる。ここではB位置とTarget判定の
    /// 両方を絶対位置指定でキャッシュから読み、作業量を対象チャンクに限定する。
    pub fn calculate_guidance_batch_from_cache(
        request: GuidanceSearchRequest,
        cache: &ObservationCache,
    ) -> Result<Vec<(i64, bool)>, AppError> {
        let empty_results = || {
            (request.observed_start..=request.observed_end)
                .map(|frame| (frame, false))
                .collect::<Vec<_>>()
        };
        if cache.seed != request.seed {
            return Err(AppError::Cache(CacheError::SeedMismatch));
        }
        if request.target.saturating_sub(request.observed_start) > MAX_PRODUCTION_LOOKAHEAD {
            return Ok(empty_results());
        }
        let Some(b_start) = request
            .observed_start
            .checked_add(request.pre_rotom_consumption)
        else {
            return Ok(empty_results());
        };
        let Some(b_count) = request
            .observed_end
            .checked_sub(request.observed_start)
            .and_then(|span| span.checked_add(1))
            .and_then(|count| usize::try_from(count).ok())
        else {
            return Ok(empty_results());
        };
        let b_values = cache.read_values(b_start, b_count)?;
        let mut planned = Vec::with_capacity(b_count);
        for (index, frame) in (request.observed_start..=request.observed_end).enumerate() {
            let Some(value) = b_values.get(index).copied() else {
                return Ok(empty_results());
            };
            let Some(plan) = crate::domain::rng::encounter_plan_with_pre_rotom_value(
                frame,
                request.offset,
                request.consider_rotom,
                request.rotom_threshold,
                request.observed_rotom_talk,
                request.pre_rotom_consumption,
                request.npc_initial_load,
                value,
            ) else {
                return Ok(empty_results());
            };
            planned.push((frame, plan.encounter_start));
        }
        let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
            return Ok(empty_results());
        };
        let max_start = planned
            .iter()
            .map(|(_, start)| *start)
            .max()
            .unwrap_or(min_start);
        let by_start: HashMap<i64, bool> = target_reachability_range_from_cache(
            cache,
            min_start..=max_start,
            request.target,
            request.models,
            request.fastest_ticks,
        )?
        .into_iter()
        .collect();
        Ok(planned
            .into_iter()
            .map(|(observed, start)| (observed, by_start.get(&start).copied().unwrap_or(false)))
            .collect())
    }

    pub fn rotom_talk_at(&self, frame: i64) -> Option<bool> {
        let seed = self.search_config().ok()?.seed;
        let position = frame.saturating_add(self.pre_rotom_consumption_value());
        let roll = self.cached_rotom_roll_at(seed, position)?;
        Some(u64::from(roll) < self.rotom_threshold_value())
    }

    fn cached_rotom_roll_at(&self, seed: u32, position: i64) -> Option<u8> {
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)?;
        match cache.read_values(position, 1) {
            Ok(values) => values.first().map(|value| (value % 100) as u8),
            Err(error) => {
                self.defer_cache_io_error(error);
                None
            }
        }
    }

    fn defer_cache_io_error(&self, error: AppError) {
        if error.is_cache_io() {
            if let Ok(mut pending) = self.deferred_cache_io_error.lock() {
                *pending = Some(error);
            }
        }
    }

    pub(crate) fn take_deferred_cache_io_error(&self) -> Option<AppError> {
        self.deferred_cache_io_error
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
    }

    fn timeline_target_from_cache(
        &self,
        seed: u32,
        start: i64,
        target: i64,
        models: usize,
    ) -> Result<Option<crate::domain::rng::TimelineTarget>, AppError> {
        match self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
        {
            Some(cache) => timeline_target_from_cache(cache, start, target, models),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::domain::rng::{
        advance_timeline, encounter_plan_with_observed_rotom_and_initial_load,
        encounter_plan_with_pre_rotom_consumption, encounter_plan_with_rotom, production_timeline,
        ModelStatus, Sfmt,
    };

    fn install_test_cache(state: &mut AppState) {
        let config = state.search_config().unwrap();
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            config.seed,
            0..=100_000,
            100_000,
            config.fps,
        )
        .ok();
        assert!(
            state.observation_cache.is_some(),
            "test SFMT cache must generate"
        );
    }

    fn main_reference_values(seed: u32, count: usize) -> Vec<u64> {
        let mut sfmt = Sfmt::new(seed);
        (0..count).map(|_| sfmt.next_u64()).collect()
    }

    fn main_reference_advance(
        values: &[u64],
        start: i64,
        models: usize,
        ticks: i64,
    ) -> Option<i64> {
        if start < 0 || ticks < 0 || models == 0 {
            return None;
        }
        let mut cursor = usize::try_from(start).ok()?;
        let mut status = ModelStatus::new(models);
        let mut frame = start;
        for _ in 0..ticks {
            let (used, _) = status.next_state_with(|| {
                let value = values[cursor];
                cursor += 1;
                value
            });
            frame = frame.checked_add(used as i64)?;
        }
        Some(frame)
    }

    fn main_reference_simulate(values: &[u64], start: i64, target: i64, models: usize) -> bool {
        let mut cursor = usize::try_from(start.max(0)).unwrap();
        let mut status = ModelStatus::new(models);
        let mut frame = start;
        let mut tick = 0;
        while frame <= target && tick < 10_000_000 {
            if frame == target {
                return true;
            }
            let (used, _) = status.next_state_with(|| {
                let value = values[cursor];
                cursor += 1;
                value
            });
            frame += used as i64;
            tick += 1;
        }
        false
    }

    fn main_reference_target_reachability_range(
        seed: u32,
        range: std::ops::RangeInclusive<i64>,
        target: i64,
        models: usize,
        fastest_ticks: Option<i64>,
    ) -> Vec<(i64, bool)> {
        if range.is_empty() || *range.start() < 0 || models == 0 {
            return Vec::new();
        }
        let end = *range.end();
        let extra = fastest_ticks
            .unwrap_or(1)
            .max(1)
            .saturating_mul(models as i64)
            .saturating_add(8) as usize;
        let last = end.max(target).max(0) as usize;
        let values = main_reference_values(seed, last.saturating_add(extra).saturating_add(1));
        range
            .map(|start| {
                let reachable = match fastest_ticks {
                    Some(ticks) => {
                        main_reference_advance(&values, start, models, ticks) == Some(target)
                    }
                    None => main_reference_simulate(&values, start, target, models),
                };
                (start, reachable)
            })
            .collect()
    }

    fn main_reference_guidance_batch(request: GuidanceSearchRequest) -> Vec<(i64, bool)> {
        if request.target.saturating_sub(request.observed_start) > MAX_PRODUCTION_LOOKAHEAD {
            return (request.observed_start..=request.observed_end)
                .map(|frame| (frame, false))
                .collect();
        }
        let mut planned = Vec::new();
        for frame in request.observed_start..=request.observed_end {
            if let Some(plan) = encounter_plan_with_observed_rotom_and_initial_load(
                request.seed,
                frame,
                request.offset,
                request.consider_rotom,
                request.rotom_threshold,
                None,
                request.npc_initial_load,
            ) {
                planned.push((frame, plan.encounter_start));
            }
        }
        let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
            return Vec::new();
        };
        let max_start = planned
            .iter()
            .map(|(_, start)| *start)
            .max()
            .unwrap_or(min_start);
        let by_start: HashMap<i64, bool> = main_reference_target_reachability_range(
            request.seed,
            min_start..=max_start,
            request.target,
            request.models,
            request.fastest_ticks,
        )
        .into_iter()
        .collect();
        planned
            .into_iter()
            .filter_map(|(observed, start)| by_start.get(&start).map(|value| (observed, *value)))
            .collect()
    }

    #[test]
    fn guidance_batch_matches_main_reference_for_normal_waiting() {
        let request = GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 10,
            observed_end: 90,
            range_start: 0,
            range_end: 1_000,
            seed: 0xB63523CA,
            target: 140,
            models: 6,
            offset: 200,
            consider_rotom: true,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 3,
            fastest_ticks: None,
        };

        assert_eq!(
            AppState::calculate_guidance_batch(request),
            main_reference_guidance_batch(request)
        );
    }

    #[test]
    fn guidance_batch_matches_main_reference_for_fastest_close() {
        let seed = 0xB63523CA;
        let models = 6;
        let base =
            encounter_plan_with_observed_rotom_and_initial_load(seed, 10, 0, true, 79, None, 3)
                .unwrap()
                .encounter_start;
        let target = advance_timeline(seed, base, models, 5).unwrap();
        let request = GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 10,
            observed_end: 90,
            range_start: 0,
            range_end: 1_000,
            seed,
            target,
            models,
            offset: 0,
            consider_rotom: true,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 3,
            fastest_ticks: Some(5),
        };

        assert_eq!(
            AppState::calculate_guidance_batch(request),
            main_reference_guidance_batch(request)
        );
    }

    #[test]
    fn guidance_batch_matches_main_reference_at_a_high_absolute_position() {
        let request = GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 50_000,
            observed_end: 50_080,
            range_start: 0,
            range_end: 100_000,
            seed: 1,
            target: 50_140,
            models: 23,
            offset: 0,
            consider_rotom: false,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 0,
            fastest_ticks: None,
        };

        assert_eq!(
            AppState::calculate_guidance_batch(request),
            main_reference_guidance_batch(request)
        );
    }

    #[test]
    fn cached_guidance_batch_matches_seed_replay_at_a_high_absolute_position() {
        let request = GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 50_000,
            observed_end: 50_080,
            range_start: 0,
            range_end: 100_000,
            seed: 1,
            target: 50_140,
            models: 23,
            offset: 0,
            consider_rotom: false,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 0,
            fastest_ticks: None,
        };
        let cache = crate::domain::rng::generate_observation_cache(
            request.seed,
            request.range_start..=request.range_end,
            request.target,
            DEFAULT_FPS,
        )
        .unwrap();

        assert_eq!(
            AppState::calculate_guidance_batch_from_cache(request, &cache).unwrap(),
            main_reference_guidance_batch(request)
        );
    }

    #[test]
    fn candidate_reachability_from_cache_matches_seed_replay() {
        let seed = 0x8A31_42C7;
        let models = 4;
        let initial =
            encounter_plan_with_observed_rotom_and_initial_load(seed, 10, 0, true, 79, None, 2)
                .unwrap()
                .encounter_start;
        let target = advance_timeline(seed, initial, models, 25).unwrap();
        let cache =
            crate::domain::rng::generate_observation_cache(seed, 0..=1_000, target, DEFAULT_FPS)
                .unwrap();

        for fastest_ticks in [None, Some(5)] {
            let request = CandidateReachabilityRequest {
                session_generation: 0,
                seed,
                target,
                models,
                offset: 0,
                consider_rotom: true,
                rotom_threshold: 79,
                observed_rotom_talk: None,
                pre_rotom_consumption: 0,
                npc_initial_load: 2,
                fastest_ticks,
                candidate_positions: vec![(0, 10), (1, 20)],
            };
            assert_eq!(
                AppState::calculate_candidate_reachability_from_cache(request.clone(), &cache)
                    .unwrap(),
                AppState::calculate_candidate_reachability(request),
            );
        }
    }

    #[test]
    fn default_beep_settings_are_half_second_and_six_times() {
        let state = AppState::default();

        assert_eq!(state.beep_interval, "0.5");
        assert_eq!(state.beep_count, "6");
    }

    #[test]
    fn observation_match_keeps_interval_and_sfmt_position_together() {
        let mut state = AppState::default();
        state.mode = Mode::Observing;
        state.intervals = vec![283, 381];
        let matched = ObservationMatch {
            start_position: 12,
            sfmt_position: 35,
            next_blink_ticks: 417,
        };
        let candidate = state.make_candidate(matched);
        assert_eq!(candidate.start_position, 12);
        assert_eq!(candidate.sfmt_position, 35);
        assert_eq!(candidate.next_blink_ticks, Some(417));
    }

    #[test]
    fn range_button_uses_target_as_end() {
        let mut state = AppState::default();
        state.target = "12345".into();
        state.range_before_target = "10000".into();
        state.set_range_from_target();
        assert_eq!(state.range_start, "2345");
        assert_eq!(state.range_end, "12345");

        state.target = "5000".into();
        state.set_range_from_target();
        assert_eq!(state.range_start, "0");
        assert_eq!(state.range_end, "5000");

        state.target = "12345".into();
        state.range_before_target = "1234".into();
        state.set_range_from_target();
        assert_eq!(state.range_start, "11111");
    }

    #[test]
    fn search_config_rejects_invalid_or_unreasonably_large_npc_counts() {
        let mut state = AppState::default();
        state.npc = "not-a-number".into();
        assert!(state.search_config().is_err());

        state.npc = (MAX_NPC_COUNT + 1).to_string();
        assert!(state.search_config().is_err());

        state.npc = MAX_NPC_COUNT.to_string();
        assert!(state.search_config().is_ok());
    }

    #[test]
    fn search_config_allows_target_far_from_observation_range_start() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.target = (MAX_PRODUCTION_LOOKAHEAD + 1).to_string();
        assert!(state.search_config().is_ok());

        state.range_start = "9000000".into();
        state.range_end = (9_000_000 + MAX_PRODUCTION_LOOKAHEAD).to_string();
        state.target = (9_000_000 + MAX_PRODUCTION_LOOKAHEAD).to_string();
        assert!(state.search_config().is_ok());
    }

    #[test]
    fn guidance_batch_short_circuits_target_beyond_current_window() {
        let request = GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 100,
            observed_end: 104,
            range_start: 0,
            range_end: MAX_FRAME,
            seed: 1,
            target: 100 + MAX_PRODUCTION_LOOKAHEAD + 1,
            models: 6,
            offset: 0,
            consider_rotom: true,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 1,
            fastest_ticks: None,
        };
        assert_eq!(
            AppState::calculate_guidance_batch(request),
            vec![
                (100, false),
                (101, false),
                (102, false),
                (103, false),
                (104, false)
            ]
        );
    }

    #[test]
    fn guidance_request_keeps_main_search_end_at_configured_range() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "1000".into();
        state.target = "5000".into();
        state.npc = "5".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 3000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(3000);

        let request = state.guidance_search_request().expect("guidance request");

        assert_eq!(request.observed_start, 3000);
        assert_eq!(request.observed_end, 3000);
    }

    #[test]
    fn production_preparation_builds_the_selected_timeline_off_the_cached_sequence() {
        let mut state = AppState::default();
        state.seed = "00000001".into();
        state.range_start = "0".into();
        state.range_end = "1000".into();
        state.target = "5000".into();
        state.npc = "5".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 1000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        install_test_cache(&mut state);
        state.recalculate_production();
        assert!(state.production_preparation_pending);
        assert!(state.production.is_none());

        let (request, cache) = state
            .production_preparation_request()
            .expect("production request");
        let prepared = AppState::calculate_production_preparation(request.clone(), cache)
            .expect("prepared production timeline");

        assert_eq!(prepared.production.target, 5000);
        assert!(state.apply_production_preparation(&request, prepared));
        assert!(!state.production_preparation_pending);
        assert_eq!(
            state.production.as_ref().map(|value| value.target),
            Some(5000)
        );
    }

    #[test]
    fn incomplete_guidance_results_do_not_advance_computed_end() {
        let mut state = AppState::default();
        let request = GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 10,
            observed_end: 12,
            range_start: 0,
            range_end: 100,
            seed: 1,
            target: 20,
            models: 1,
            offset: 0,
            consider_rotom: false,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 0,
            fastest_ticks: None,
        };

        state.apply_guidance_results(request, vec![(10, false), (12, true)]);

        assert_eq!(state.guidance.computed_until(), None);
        assert!(state.guidance.is_empty());
    }

    #[test]
    fn guidance_batch_covers_current_to_target_even_when_observation_range_ended() {
        let request = GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 3_000,
            observed_end: 5_000,
            range_start: 0,
            range_end: 1_000,
            seed: 1,
            target: 5_000,
            models: 1,
            offset: 0,
            consider_rotom: false,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 0,
            fastest_ticks: None,
        };
        let results = AppState::calculate_guidance_batch(request);
        assert_eq!(results.len(), 2_001);
        assert_eq!(results.first().map(|(frame, _)| *frame), Some(3_000));
        assert_eq!(results.last().map(|(frame, _)| *frame), Some(5_000));
    }

    #[test]
    fn fastest_guidance_applies_offset_ticks_once() {
        let seed = 1;
        let models = 2;
        let query = ProductionQuery {
            seed,
            target: 0,
            models,
            offset: 0,
            consider_rotom: true,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 1,
            fastest_ticks: Some(5),
        };
        let base = query.base_encounter_start(0).unwrap();
        let target = advance_timeline(seed, base, models, 5).unwrap();
        let results = AppState::calculate_guidance_batch(GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 0,
            observed_end: 0,
            range_start: 0,
            range_end: 100,
            seed,
            target,
            models,
            offset: 0,
            consider_rotom: true,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 1,
            fastest_ticks: Some(5),
        });
        assert_eq!(results, vec![(0, true)]);
    }

    #[test]
    fn guidance_snapshot_exposes_next_circle_after_worker_result() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "5000".into();
        state.npc = "5".into();
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 3000,
            sfmt_position: 3000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        let request = state.guidance_search_request().unwrap();
        let results = AppState::calculate_guidance_batch(request);
        state.apply_guidance_results(request, results);
        assert!(state
            .hatch_guidance()
            .unwrap()
            .unwrap()
            .next_circle_frame
            .is_some());
    }

    #[test]
    fn guidance_snapshot_keeps_current_result_visible_while_future_chunks_are_pending() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "5000".into();
        state.npc = "5".into();
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 3000,
            sfmt_position: 3000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];

        let request = state.guidance_search_request().unwrap();
        let first_result = AppState::calculate_guidance_batch(request)[0];
        let mut first_request = request;
        first_request.observed_end = first_request.observed_start;
        state.apply_guidance_results(first_request, vec![first_result]);

        let guidance = state.hatch_guidance().unwrap().unwrap();
        assert_eq!(guidance.current_reachability, Some(first_result.1));
        assert!(state.guidance_search_pending());
    }

    #[test]
    fn fastest_close_marks_target_unreachable_after_extended_target_window_finishes() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "6000".into();
        state.npc = "0".into();
        state.fastest_close = true;
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=10_000,
            10_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 4800,
            sfmt_position: 4800,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(4800);

        let request = state
            .guidance_search_request()
            .expect("Target window must remain searchable");
        let results = (request.observed_start..=request.observed_end)
            .map(|frame| (frame, false))
            .collect();
        state.apply_guidance_results(request, results);

        let guidance = state
            .hatch_guidance()
            .expect("guidance calculation")
            .expect("guidance snapshot");
        assert_eq!(guidance.current_reachability, Some(false));
        assert!(guidance.target_unreachable);
    }

    #[test]
    fn fastest_close_grace_uses_seed_when_target_is_beyond_observation_cache() {
        let mut state = AppState::default();
        state.range_end = "5000".into();
        state.target = "10000".into();
        state.npc = "0".into();
        state.fastest_close = true;
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 4800,
            sfmt_position: 4800,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(4800);
        let request = state.guidance_search_request().unwrap();
        let results = AppState::calculate_guidance_batch(request);
        state.apply_guidance_results(request, results);
        let guidance = state.hatch_guidance().unwrap().unwrap();
        assert!(guidance.next_circle_frame.is_some());
        assert!(guidance.grace_seconds.is_some());
    }

    #[test]
    fn guidance_marks_targets_outside_the_search_window_without_calling_them_unreachable() {
        for fastest_close in [false, true] {
            let mut state = AppState::default();
            state.range_start = "0".into();
            state.range_end = "5000".into();
            state.target = "110000".into();
            state.npc = "0".into();
            state.fastest_close = fastest_close;
            state.observation_cache = crate::domain::rng::generate_observation_cache(
                0xB63523CA,
                0..=5_000,
                5_000,
                state.fps_value(),
            )
            .ok();
            state.candidates = vec![TimelineCandidate {
                start_position: 4800,
                sfmt_position: 4800,
                next_blink_ticks: Some(130),
                target_reachable: false,
            }];
            state.next_blink_index = Some(4800);

            let guidance = state
                .hatch_guidance()
                .expect("guidance calculation")
                .expect("guidance snapshot");
            assert!(guidance.search_out_of_range);
            assert!(!guidance.target_unreachable);
        }
    }

    #[test]
    fn normal_guidance_keeps_near_target_inside_the_current_window() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "6000".into();
        state.npc = "0".into();
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 4800,
            sfmt_position: 4800,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(4800);

        let guidance = state
            .hatch_guidance()
            .expect("guidance calculation")
            .expect("guidance snapshot");
        assert!(!guidance.search_out_of_range);
        assert!(!guidance.target_unreachable);
    }

    #[test]
    fn normal_guidance_marks_a_target_before_current_as_unreachable() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "4000".into();
        state.npc = "0".into();
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 4800,
            sfmt_position: 4800,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(4800);

        let guidance = state
            .hatch_guidance()
            .expect("guidance calculation")
            .expect("guidance snapshot");
        assert!(!guidance.search_out_of_range);
        assert!(guidance.target_unreachable);
    }

    #[test]
    fn stale_guidance_results_are_rejected_by_state() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "5000".into();
        state.npc = "5".into();
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 3000,
            sfmt_position: 3000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];

        let current = state.guidance_search_request().unwrap();
        let mut stale = current;
        stale.seed ^= 1;
        let result = vec![(current.observed_start, false)];
        state.apply_guidance_results(stale, result);

        assert!(state.guidance.is_empty());
        assert_eq!(state.guidance.computed_until(), None);
    }

    #[test]
    fn guidance_snapshot_finishes_grace_at_a_known_no_result() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "5000".into();
        state.npc = "5".into();
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 3000,
            sfmt_position: 3000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.guidance.record_reachability(3000, true);
        state.guidance.record_reachability(3001, false);
        state.guidance.mark_computed_until(3001);

        let guidance = state.hatch_guidance().unwrap().unwrap();
        assert_eq!(guidance.current_reachability, Some(true));
        assert!(guidance.grace_seconds.is_some());
        assert!(state.guidance_search_pending());
    }

    #[test]
    fn fastest_guidance_marks_theoretical_outside_position_as_no_result() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "10000".into();
        state.target = "5000".into();
        state.npc = "0".into();
        state.use_offset = true;
        state.encounter_offset = "1000".into();
        state.fastest_close = true;
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=10_000,
            10_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 3000,
            sfmt_position: 3000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];

        let guidance = state.hatch_guidance().unwrap().unwrap();
        assert_eq!(guidance.current_reachability, Some(false));
        assert!(state.guidance_search_pending());
    }

    #[test]
    fn fastest_close_near_target_keeps_outside_current_position_and_searches_target_window() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "10000".into();
        state.target = "6000".into();
        state.npc = "0".into();
        state.fastest_close = true;
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=10_000,
            10_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 4800,
            sfmt_position: 4800,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(4800);

        let guidance = state
            .hatch_guidance()
            .expect("guidance calculation")
            .expect("guidance snapshot");
        assert_eq!(guidance.current_reachability, Some(false));
        assert!(state.guidance_search_request().is_some());
        assert!(state.guidance_search_pending());
    }

    #[test]
    fn fastest_close_searches_target_window_when_observation_range_has_ended() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "6000".into();
        state.npc = "0".into();
        state.fastest_close = true;
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=10_000,
            10_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 4800,
            sfmt_position: 4800,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(4800);

        let request = state
            .guidance_search_request()
            .expect("Target window must remain searchable");
        assert!(request.observed_end > 5000);
        assert!(state.guidance_search_pending());
    }

    #[test]
    fn encounter_offset_rejects_values_above_the_supported_limit() {
        let mut state = AppState::default();
        state.use_offset = true;
        state.encounter_offset = (MAX_ENCOUNTER_OFFSET_FRAMES + 1).to_string();
        assert!(state.encounter_offset_error().is_some());

        state.encounter_offset = MAX_ENCOUNTER_OFFSET_FRAMES.to_string();
        assert!(state.encounter_offset_error().is_none());

        state.encounter_offset.clear();
        assert!(state.encounter_offset_error().is_none());
    }

    #[test]
    fn offset_correction_converts_strict_timeline_ticks_to_display_frames() {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "1000".into();
        state.npc = "1".into();
        state.use_offset = true;
        state.encounter_offset = "10".into();
        state.fastest_close = true;
        let config = state.search_config().unwrap();
        install_test_cache(&mut state);
        let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
        // 設定した10FオフセットはTimelineの5tickに相当する。Targetと実測Frameは
        // B+ロトムから続く同じ経路から求め、オフセット境界でModelStatusを初期化し直さない。
        let target =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
        let actual =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 7).unwrap();
        state.target = target.to_string();
        state.actual_target_frame = actual.to_string();
        assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 14);
    }

    #[test]
    fn offset_correction_uses_the_selected_circle_rotom_prediction() {
        let mut state = AppState::default();
        state.range_end = "1000".into();
        state.npc = "1".into();
        state.use_offset = true;
        state.encounter_offset = "10".into();
        let config = state.search_config().unwrap();
        install_test_cache(&mut state);
        let predicted = state.rotom_talk_at(0).unwrap();
        let predicted_plan = encounter_plan_with_observed_rotom(
            config.seed,
            0,
            0,
            true,
            state.rotom_threshold_value(),
            Some(predicted),
        )
        .unwrap();
        let target = advance_timeline(
            config.seed,
            predicted_plan.encounter_start,
            config.npc_models,
            5,
        )
        .unwrap();
        let actual = advance_timeline(
            config.seed,
            predicted_plan.encounter_start,
            config.npc_models,
            7,
        )
        .unwrap();
        state.target = target.to_string();
        state.observed_rotom_talk = Some(!predicted);
        state.actual_target_frame = actual.to_string();

        assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 6);
    }

    #[test]
    fn correction_adjusts_existing_offset_by_timeline_difference() {
        let mut state = AppState::default();
        state.seed = "B63523CA".into();
        state.target = "3000".into();
        state.npc = "5".into();
        state.use_offset = true;
        state.encounter_offset = "216".into();
        state.correction_source_offset = Some(216);
        state.actual_target_frame = "3003".into();
        install_test_cache(&mut state);
        // このseedでは○Frame 692に対し、Target=3000はtick508、実測Frame=3003は
        // tick509となり、表示Frameで2Fのずれになる。
        assert_eq!(state.suggested_offset_for_actual_from(692).unwrap(), 214);
        state.encounter_offset = "214".into();
        assert_eq!(state.suggested_offset_for_actual_from(692).unwrap(), 214);
    }

    #[test]
    fn correction_offset_uses_target_actual_display_frame_drift() {
        let mut state = AppState::default();
        state.use_offset = true;
        state.encounter_offset = "100".into();
        // プレビューは実際に開始した○を使う。Targetを仮の開始位置にせず、実在する
        // 開始位置からTimelineを生成する。
        state.npc = "5".into();
        let source_plan = encounter_plan_with_rotom(1, 0, 0, true).unwrap();
        let source_target = advance_timeline(1, source_plan.encounter_start, 6, 5).unwrap();
        let source_actual = advance_timeline(1, source_plan.encounter_start, 6, 7).unwrap();
        state.target = source_target.to_string();
        state.actual_target_frame = source_actual.to_string();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: None,
            target_reachable: false,
        }];
        install_test_cache(&mut state);
        let preview = state.correction_preview_from(0).unwrap();
        assert_eq!(preview.suggested_offset, 100 - 4);
        assert_eq!(preview.timeline_start, source_plan.encounter_start);
        assert!(preview.target_on_timeline);
        assert!(preview.actual_on_timeline);
    }

    #[test]
    fn correction_preview_reports_a_missing_seed_cache_as_calculation_unavailable() {
        let mut state = AppState::default();
        state.actual_target_frame = "1000".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: None,
            target_reachable: false,
        }];

        let error = state.correction_preview_from(0).unwrap_err();
        assert!(matches!(error, CorrectionPreviewError::CacheSeedMismatch));
    }

    #[test]
    fn hatch_correction_and_production_share_consumption_order() {
        let mut state = AppState::default();
        state.seed = "00000001".into();
        state.range_end = "1000".into();
        state.npc = "3".into();
        state.consider_pre_rotom_consumption = true;
        state.pre_rotom_consumption = "5".into();
        state.consider_npc_initial_load = true;
        state.npc_initial_load = "4".into();
        state.use_offset = true;
        state.encounter_offset = "200".into();
        state.fastest_close = false;
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: None,
            target_reachable: true,
        }];

        let config = state.search_config().unwrap();
        install_test_cache(&mut state);
        let expected = encounter_plan_with_pre_rotom_consumption(
            config.seed,
            0,
            0,
            true,
            state.rotom_threshold_value(),
            None,
            5,
            4,
        )
        .unwrap();
        let correction = state.correction_plan(&config, 0).unwrap();
        let production = state.article_plan_for_selected().unwrap();

        assert_eq!(correction.b_position, expected.b_position);
        assert_eq!(correction.rotom_consumption, expected.rotom_consumption);
        assert_eq!(correction.encounter_start, expected.encounter_start);
        assert_eq!(production.b_position, expected.b_position);
        assert_eq!(production.rotom_consumption, expected.rotom_consumption);
        assert_eq!(production.encounter_start, expected.encounter_start);
    }

    #[test]
    fn correction_keeps_offset_when_actual_is_target_step_end() {
        let mut state = AppState::default();
        state.use_offset = true;
        state.encounter_offset = "100".into();
        state.npc = "5".into();
        let source_plan = encounter_plan_with_rotom(1, 0, 0, true).unwrap();
        let target = source_plan.encounter_start + 1;
        let actual = advance_timeline(1, source_plan.encounter_start, 6, 1).unwrap();
        state.target = target.to_string();
        state.actual_target_frame = actual.to_string();
        install_test_cache(&mut state);

        assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 100);
    }

    #[test]
    fn fastest_correction_can_compare_frame_before_offset_landing() {
        let mut state = AppState::default();
        state.fastest_close = true;
        state.use_offset = true;
        state.encounter_offset = "10".into();
        state.npc = "1".into();
        let config = state.search_config().unwrap();
        let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
        let target =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
        let actual =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 4).unwrap();
        assert!(actual < target);
        state.target = target.to_string();
        state.actual_target_frame = actual.to_string();
        install_test_cache(&mut state);

        // オフセット着地後も同じModelStatus状態を継続する。新しい初期化境界ではなく、
        // 正負どちらの補正方向も有効である。
        assert!(state.suggested_offset_for_actual_from(0).is_ok());
    }

    #[test]
    fn correction_still_returns_preview_when_direction_would_go_below_zero() {
        let mut state = AppState::default();
        state.npc = "1".into();
        state.use_offset = false;
        let config = state.search_config().unwrap();
        let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
        let earlier =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 1).unwrap();
        let later =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 2).unwrap();
        install_test_cache(&mut state);

        // 通常待機で実測がTargetより後ろでも、負のオフセットを示すだけで補正操作は
        // 0Fの適用可能なプレビューを返す。
        state.fastest_close = false;
        state.target = earlier.to_string();
        state.actual_target_frame = later.to_string();
        assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 0);

        // 最速閉じで実測がTargetより前の場合も、同じ下限処理を適用する。
        state.fastest_close = true;
        state.target = later.to_string();
        state.actual_target_frame = earlier.to_string();
        assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 0);
    }

    #[test]
    fn fastest_close_reachability_uses_offset_as_timeline_time() {
        let mut state = AppState::default();
        state.npc = "1".into();
        state.use_offset = true;
        state.encounter_offset = "10".into();
        state.fastest_close = true;
        state.range_start = "0".into();
        state.range_end = "100".into();
        let config = state.search_config().unwrap();
        let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
        let target =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
        state.target = target.to_string();
        install_test_cache(&mut state);
        assert_eq!(state.table_target_reachable(0), Some(true));
    }

    #[test]
    fn fastest_close_drift_entry_stops_blink_timer_without_offset() {
        let mut state = AppState::default();
        state.mode = Mode::Ready;
        state.fastest_close = true;
        state.actual_target_frame = "1000".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: Some(130),
            target_reachable: true,
        }];
        state.fast_close_circle_frames = vec![0];
        state.next_blink_index = Some(0);
        state.next_blink = Some(Instant::now() + Duration::from_secs(1));
        install_test_cache(&mut state);

        assert!(state.finish_fastest_close_for_correction());
        assert!(matches!(state.mode, Mode::Finished));
        assert!(state.next_blink.is_none());
        assert_eq!(state.correction_start_candidates(), vec![0]);
        assert_eq!(state.correction_target_frame, Some(1000));
        assert!(state.correction_preview_from(0).is_ok());

        // Enter後に補正関連の値を変更しても、停止した瞬き／本番タイマーを再開しない。
        state.encounter_offset = "2".into();
        state.recalculate_production();
        assert!(matches!(state.mode, Mode::Finished));
        assert!(state.timer_start.is_none());
        assert!(state.next_blink.is_none());
    }

    #[test]
    fn fastest_close_uses_recorded_circles_when_entered_after_target_window() {
        let mut state = AppState::default();
        state.mode = Mode::Ready;
        state.fastest_close = true;
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 480,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(480);

        assert!(!state.can_finish_fastest_close_for_correction());
        assert!(!state.finish_fastest_close_for_correction());
        assert!(matches!(state.mode, Mode::Ready));
        assert!(state.correction_start_candidates().is_empty());

        state.fast_close_circle_frames = vec![478, 482];
        assert!(state.can_finish_fastest_close_for_correction());
        assert!(state.finish_fastest_close_for_correction());
        assert_eq!(state.correction_start_candidates(), vec![478, 482]);
        assert_eq!(state.correction_source_frame, Some(478));
        assert_eq!(state.next_blink_index, Some(478));
    }

    #[test]
    fn session_phase_enters_correction_without_restarting_timers() {
        let mut state = AppState::default();
        assert_eq!(state.session_phase(), SessionPhase::Input);
        assert!(state.can_edit_observation_settings());
        state.mode = Mode::Ready;
        assert_eq!(state.session_phase(), SessionPhase::TimelineReady);
        assert!(!state.can_edit_observation_settings());
        state.enter_correction_mode();
        assert_eq!(state.session_phase(), SessionPhase::Correction);
        assert!(!state.can_edit_general_settings());
        assert!(state.can_edit_correction_inputs());
        assert!(state.timer_start.is_none());
    }

    #[test]
    fn fastest_close_guidance_bounds_use_maximum_per_step_consumption() {
        let mut state = AppState::default();
        state.target = "3000".into();
        state.range_start = "0".into();
        state.range_end = "10000".into();
        state.npc = "5".into();
        state.use_offset = true;
        state.encounter_offset = "200".into();
        state.fastest_close = true;
        state.consider_rotom_talk = true;

        let config = state.search_config().unwrap();
        assert_eq!(
            state.fastest_close_guidance_bounds(&config),
            Some((2297, 3098, true))
        );
    }

    #[test]
    fn fastest_close_bounds_treat_zero_as_the_lower_frame_limit() {
        let mut state = AppState::default();
        state.target = "100".into();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.npc = "0".into();
        state.use_offset = true;
        state.encounter_offset = "122".into();
        state.fastest_close = true;

        let config = state.search_config().unwrap();
        let (_, _, fully_covered) = state.fastest_close_guidance_bounds(&config).unwrap();
        assert!(fully_covered);
    }

    #[test]
    fn correction_uses_current_circle_for_additional_wait_even_if_target_is_unreachable() {
        let mut state = AppState::default();
        state.mode = Mode::Ready;
        state.fastest_close = false;
        state.range_end = "100".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        let config = state.search_config().unwrap();
        let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
        state.target = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 3)
            .unwrap()
            .to_string();
        assert_eq!(state.correction_start_candidates(), vec![0]);
    }

    #[test]
    fn correction_uses_all_observation_candidates_when_current_position_is_ambiguous() {
        let mut state = AppState::default();
        state.fast_close_circle_frames.clear();
        state.candidates = vec![
            TimelineCandidate {
                start_position: 0,
                sfmt_position: 692,
                next_blink_ticks: Some(130),
                target_reachable: false,
            },
            TimelineCandidate {
                start_position: 1,
                sfmt_position: 694,
                next_blink_ticks: Some(130),
                target_reachable: false,
            },
        ];

        assert_eq!(state.correction_start_candidates(), vec![692, 694]);
    }

    #[test]
    fn correction_source_selection_keeps_all_fastest_close_starts() {
        let mut state = AppState::default();
        state.fastest_close = true;
        state.fast_close_circle_frames = vec![10, 12, 14];

        assert_eq!(state.correction_start_candidates(), vec![10, 12, 14]);
        assert_eq!(state.selected_correction_start(), Some(10));
        assert!(state.set_correction_source_index(2));
        assert_eq!(state.correction_source_index(), 2);
        assert_eq!(state.selected_correction_start(), Some(14));
    }

    #[test]
    fn correction_source_selection_updates_rotom_prediction_without_touching_guidance() {
        let mut state = AppState::default();
        state.fastest_close = true;
        state.fast_close_circle_frames = vec![0, 1];
        state.guidance.record_reachability(123, true);
        install_test_cache(&mut state);

        assert!(state.set_correction_source_index(1));
        let predicted = state.predicted_rotom_talk_for_correction_source();
        assert!(predicted.is_some());
        assert!(state.sync_observed_rotom_to_correction_source());
        assert_eq!(state.observed_rotom_talk, predicted);
        assert!(state.guidance.contains_reachable(123));
    }

    #[test]
    fn reported_fastest_close_case_uses_each_start_position() {
        let expected = [
            (808, 10, 813, 1028),
            (809, 47, 814, 1028),
            (810, 2, 815, 1028),
        ];
        for (start, expected_roll, expected_encounter_start, expected_after_offset) in expected {
            assert_eq!(rotom_roll_at(1, start), Some(expected_roll));
            let plan = encounter_plan_with_pre_rotom_consumption(1, start, 0, true, 70, None, 0, 3)
                .expect("reported case must produce an encounter plan");
            assert_eq!(plan.rotom_consumption, 2);
            assert_eq!(plan.encounter_start, expected_encounter_start);
            assert_eq!(
                advance_timeline(1, plan.encounter_start, 4, 61),
                Some(expected_after_offset)
            );
        }

        let guidance = AppState::calculate_guidance_batch(GuidanceSearchRequest {
            session_generation: 0,
            observed_start: 808,
            observed_end: 810,
            range_start: 808,
            range_end: 810,
            seed: 1,
            target: 1028,
            models: 4,
            offset: 122,
            consider_rotom: true,
            rotom_threshold: 70,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 3,
            fastest_ticks: Some(61),
        });
        assert_eq!(guidance, vec![(808, true), (809, true), (810, true)]);
    }

    #[test]
    fn fastest_close_without_recorded_circle_skips_rotom_correction() {
        let mut state = AppState::default();
        state.fastest_close = true;
        state.fast_close_circle_frames.clear();
        assert!(state.correction_start_candidates().is_empty());
        assert!(state.rotom_correction_from(0, 0).is_none());
    }

    #[test]
    fn rotom_correction_uses_only_start_roll_and_observed_talk() {
        let mut state = AppState::default();
        state.fastest_close = true;
        state.fast_close_circle_frames = vec![0];
        install_test_cache(&mut state);
        let predicted = state.rotom_talk_at(0).unwrap();
        state.observed_rotom_talk = Some(!predicted);

        let correction = state
            .rotom_correction_from(0, MAX_FRAME)
            .expect("opposite observed talk must produce a threshold correction");
        let roll = u64::from(correction.roll);
        let expected = if correction.observed_talk {
            roll + 1
        } else {
            roll
        }
        .min(100);
        assert_eq!(correction.suggested_threshold, expected);
    }

    #[test]
    fn correction_requires_actual_frame_and_valid_conditions() {
        let mut state = AppState::default();
        assert!(!state.correction_inputs_complete());
        state.actual_target_frame = "100".into();
        assert!(state.correction_inputs_complete());
        state.npc = "not-a-number".into();
        assert!(!state.correction_inputs_complete());
    }

    #[test]
    fn additional_wait_enter_starts_offset_then_target_timer() {
        let mut state = AppState::default();
        state.mode = Mode::Ready;
        state.fastest_close = false;
        state.use_offset = true;
        state.encounter_offset = "60".into();
        state.range_start = "0".into();
        state.range_end = "100".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: Some(130),
            target_reachable: true,
        }];
        let config = state.search_config().unwrap();
        let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
        let target =
            advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
        state.target = target.to_string();
        install_test_cache(&mut state);
        state.mark_production_preparation_pending();
        let (request, cache) = state
            .production_preparation_request()
            .expect("production request");
        let prepared = AppState::calculate_production_preparation(request.clone(), cache)
            .expect("prepared production timeline");
        assert!(state.apply_production_preparation(&request, prepared));
        let expected_tick =
            production_timeline(config.seed, plan.encounter_start, target, config.npc_models)
                .target_tick
                .unwrap();

        state.begin_encounter();

        assert!(matches!(state.mode, Mode::Countdown));
        assert!(matches!(
            state.article_timer_stage,
            Some(ArticleTimerStage::Offset)
        ));
        let offset_seconds = game_frames_to_seconds(60.0, DEFAULT_FPS);
        let target_seconds = blink_ticks_to_seconds(expected_tick, DEFAULT_FPS);
        assert!((state.article_timer_seconds - offset_seconds).abs() < f64::EPSILON);
        assert_eq!(state.article_next_seconds, Some(target_seconds));

        state.article_timer_start = Some(Instant::now() - Duration::from_secs(2));
        assert_eq!(state.tick_article_timer(), Some(ArticleTimerStage::Offset));
        assert_eq!(state.article_timer_stage, Some(ArticleTimerStage::ToTarget));
        assert!((state.article_timer_seconds - target_seconds).abs() < f64::EPSILON);
        assert_eq!(state.article_next_seconds, None);
    }

    #[test]
    fn hatch_timer_catches_up_after_fifty_million_frame_virtual_elapsed() {
        let mut state = AppState::default();
        let start = Instant::now();
        let duration_seconds = game_frames_to_seconds(MAX_FRAME as f64, state.fps_value());
        let virtual_now = start + Duration::from_secs_f64(duration_seconds);
        state.mode = Mode::Production;
        state.timer_start = Some(start);
        state.timer_seconds = duration_seconds;

        assert_eq!(state.remaining_at(virtual_now), 0.0);
        assert!(state.finish_production_if_elapsed(virtual_now));
        assert!(state.is_finished());
        assert_eq!(state.remaining_at(virtual_now), 0.0);
    }

    #[test]
    fn hatch_article_timer_catches_up_both_stages_after_long_virtual_elapsed() {
        let mut state = AppState::default();
        let start = Instant::now();
        let long_wait = game_frames_to_seconds(MAX_FRAME as f64, state.fps_value());
        let virtual_now = start + Duration::from_secs_f64(long_wait);
        state.mode = Mode::Countdown;
        state.article_timer_stage = Some(ArticleTimerStage::Offset);
        state.article_timer_start = Some(start);
        state.article_timer_seconds = game_frames_to_seconds(120.0, state.fps_value());
        state.article_next_seconds = Some(30.0);

        assert_eq!(
            state.tick_article_timer_at(virtual_now),
            Some(ArticleTimerStage::Offset)
        );
        assert_eq!(state.article_timer_stage, Some(ArticleTimerStage::ToTarget));
        assert_eq!(
            state.tick_article_timer_at(virtual_now),
            Some(ArticleTimerStage::ToTarget)
        );
        assert!(state.is_finished());
        assert_eq!(state.article_timer_start_time(), None);
    }

    #[test]
    fn observed_rotom_talk_does_not_change_configured_threshold() {
        let mut state = AppState::default();
        install_test_cache(&mut state);
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: None,
            target_reachable: false,
        }];
        let roll = rotom_roll_at(1, 0).unwrap();
        let observed = u64::from(roll) >= 79;
        let configured_prediction = u64::from(roll) < 79;

        state.set_observed_rotom_talk(observed);

        assert_eq!(state.rotom_threshold_value(), 79);
        assert_eq!(state.predicted_rotom_talk(), Some(configured_prediction));
    }

    #[test]
    fn observed_rotom_talk_keeps_guidance_results() {
        let mut state = AppState::default();
        state.guidance.record_reachability(123, true);

        state.set_observed_rotom_talk(true);

        assert!(state.guidance.contains_reachable(123));
    }

    #[test]
    fn normal_timer_sums_zero_or_more_offsets() {
        let mut timer = NormalTimerState::default();
        timer.wait_frames = "120".into();
        assert_eq!(timer.total_frames().unwrap(), 120);

        timer.offsets = vec!["5".into(), "2".into(), "10".into()];
        assert_eq!(timer.total_frames().unwrap(), 137);
    }

    #[test]
    fn normal_timer_runs_offsets_top_to_bottom_then_actual_wait() {
        let mut timer = NormalTimerState::default();
        timer.wait_frames = "800".into();
        timer.offsets = vec!["200".into(), "100".into()];

        timer.start(100.0).unwrap();
        assert_eq!(timer.current_segment(), Some(NormalTimerSegment::Offset(0)));
        assert!((timer.timer_seconds - 2.0).abs() < f64::EPSILON);
        assert_eq!(timer.next_segment_seconds(100.0), Some(1.0));

        timer.timer_start = Some(Instant::now() - Duration::from_secs(3));
        assert!(timer.advance_segment(100.0));
        assert_eq!(timer.current_segment(), Some(NormalTimerSegment::Offset(1)));
        assert!((timer.timer_seconds - 1.0).abs() < f64::EPSILON);

        assert!(timer.advance_segment(100.0));
        assert_eq!(timer.current_segment(), Some(NormalTimerSegment::Wait));
        assert!((timer.timer_seconds - 8.0).abs() < f64::EPSILON);
        assert_eq!(timer.next_segment_seconds(100.0), None);
    }

    #[test]
    fn normal_timer_ignores_disabled_offsets_but_keeps_their_values() {
        let mut timer = NormalTimerState::default();
        timer.wait_frames = "800".into();
        timer.offsets = vec!["200".into(), "100".into()];
        timer.offset_enabled = vec![true, false];

        assert_eq!(timer.total_frames().unwrap(), 1_000);
        assert_eq!(
            timer.segment_plan().unwrap(),
            vec![
                (NormalTimerSegment::Offset(0), 200),
                (NormalTimerSegment::Wait, 800)
            ]
        );
    }

    #[test]
    fn normal_timer_supports_zero_offsets_and_rejects_negative_segments() {
        let mut timer = NormalTimerState::default();
        timer.wait_frames = "800".into();
        assert_eq!(
            timer.segment_plan().unwrap(),
            vec![(NormalTimerSegment::Wait, 800)]
        );

        timer.offsets = vec!["-2".into()];
        assert!(timer.start(DEFAULT_FPS).is_err());
    }

    #[test]
    fn timeline_display_position_follows_predicted_blinks() {
        let mut state = AppState::default();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 100,
            next_blink_ticks: Some(300),
            target_reachable: true,
        }];
        state.adjust = 2;
        state.next_blink_index = Some(100);
        assert_eq!(state.timeline_display_position(), Some(100));

        state.next_blink_index = Some(101);
        assert_eq!(state.timeline_display_position(), Some(101));
    }

    #[test]
    fn multiple_candidates_do_not_start_or_display_a_blink_timer() {
        let mut state = AppState::default();
        state.observed.push(Instant::now());
        state.candidates = vec![
            TimelineCandidate {
                start_position: 0,
                sfmt_position: 100,
                next_blink_ticks: Some(300),
                target_reachable: true,
            },
            TimelineCandidate {
                start_position: 10,
                sfmt_position: 110,
                next_blink_ticks: Some(200),
                target_reachable: true,
            },
        ];

        state.schedule_blink();

        assert!(state.next_blink.is_none());
        assert!(state.next_blink_index.is_none());
        assert_eq!(state.timeline_display_position(), None);
    }

    #[test]
    fn blink_timer_uses_observation_anchor_when_cache_is_unavailable() {
        let mut state = AppState::default();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 100,
            next_blink_ticks: Some(300),
            target_reachable: true,
        }];
        let observed_at = Instant::now() - Duration::from_secs(1);
        state.observed.push(observed_at);

        state.schedule_blink();

        let deadline = state.next_blink.expect("blink deadline");
        let expected =
            300.0 * crate::domain::rng::GAME_FRAMES_PER_BLINK_TICK as f64 / state.fps_value();
        assert!((deadline.duration_since(observed_at).as_secs_f64() - expected).abs() < 0.001);
        assert!(deadline < Instant::now() + Duration::from_secs_f64(expected));
    }

    #[test]
    fn blink_timer_connects_prediction_after_actual_observed_elapsed_time() {
        let mut state = AppState::default();
        let fps = state.fps_value();
        let cache = crate::domain::rng::generate_observation_cache(1, 0..=4, 4, fps).unwrap();
        let expected_seconds = cache.interval_seconds_at(2).unwrap().unwrap();
        let first_observed_at = Instant::now() - Duration::from_secs(3);
        let last_observed_at = first_observed_at + Duration::from_secs_f64(1.234);
        state.observation_cache = Some(cache);
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 2,
            next_blink_ticks: Some(130),
            target_reachable: true,
        }];
        state.observed.push(first_observed_at);
        state.observed.push(last_observed_at);
        state.schedule_blink();

        let deadline = state.next_blink.expect("blink deadline");
        assert!(
            (deadline.duration_since(last_observed_at).as_secs_f64() - expected_seconds).abs()
                < 0.001
        );
    }

    #[test]
    fn predicted_blink_uses_the_next_random_value_after_each_beep() {
        let mut state = AppState::default();
        let fps = state.fps_value();
        let cache = crate::domain::rng::generate_observation_cache(1, 0..=4, 4, fps).unwrap();
        let expected_seconds = cache.interval_seconds_at(1).unwrap().unwrap();
        state.observation_cache = Some(cache);
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: Some(130),
            target_reachable: true,
        }];
        let previous_deadline = Instant::now() + Duration::from_secs(10);
        state.next_blink = Some(previous_deadline);
        state.next_blink_index = Some(0);

        state.advance_predicted_blink();

        let next_deadline = state.next_blink.expect("next blink deadline");
        assert_eq!(state.next_blink_index, Some(1));
        assert!(
            (next_deadline
                .duration_since(previous_deadline)
                .as_secs_f64()
                - expected_seconds)
                .abs()
                < 0.001
        );
    }

    #[test]
    fn predicted_blink_keeps_known_guidance_while_production_prepares() {
        let mut state = AppState::default();
        state.seed = "00000001".into();
        state.range_start = "0".into();
        state.range_end = "100".into();
        state.target = "80".into();
        state.npc = "1".into();
        state.observation_cache =
            crate::domain::rng::generate_observation_cache(1, 0..=200, 200, state.fps_value()).ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 0,
            next_blink_ticks: Some(130),
            target_reachable: true,
        }];
        state.next_blink = Some(Instant::now() + Duration::from_secs(10));
        state.next_blink_index = Some(0);
        state.record_guidance_reachability(1, false);
        state.record_guidance_reachability(2, false);
        state.record_guidance_reachability(3, true);
        state.guidance.mark_computed_until(3);

        state.advance_predicted_blink();

        assert!(state.production_preparation_pending);
        assert_eq!(state.next_blink_index, Some(1));
        assert_eq!(state.guidance.reachability(1), Some(false));
        assert_eq!(state.guidance.reachability(3), Some(true));
        assert_eq!(
            state
                .hatch_guidance()
                .expect("guidance calculation")
                .and_then(|guidance| guidance.next_circle_frame),
            Some(3)
        );
    }

    #[test]
    fn enter_during_production_preparation_starts_when_the_worker_result_arrives() {
        let mut state = AppState::default();
        state.mode = Mode::Ready;
        state.fastest_close = false;
        state.use_offset = false;
        state.range_start = "0".into();
        state.range_end = "1000".into();
        state.target = "5000".into();
        state.npc = "5".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 1000,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        install_test_cache(&mut state);
        let config = state.search_config().expect("valid hatch conditions");
        let predicted_rotom = state
            .predicted_rotom_talk()
            .expect("cached Rotom prediction");
        let resolved_plan = state
            .plan_for_encounter(
                config.seed,
                1000,
                state.encounter_offset_value(),
                state.consider_rotom_talk,
                state.rotom_threshold_value(),
                Some(predicted_rotom),
            )
            .expect("plan with the Enter-time Rotom prediction");
        state.target = advance_timeline(
            config.seed,
            resolved_plan.encounter_start,
            config.npc_models,
            5,
        )
        .expect("reachable test Target")
        .to_string();
        state.mark_production_preparation_pending();
        let (stale_request, cache) = state
            .production_preparation_request()
            .expect("production request");
        let stale_prepared =
            AppState::calculate_production_preparation(stale_request.clone(), cache)
                .expect("prepared production timeline");

        state.begin_encounter();
        assert!(state.encounter_start_requested);
        assert!(state.is_ready());
        assert!(!state.apply_production_preparation(&stale_request, stale_prepared));
        assert!(state.production_preparation_pending);
        assert!(state.encounter_start_requested);

        let (current_request, cache) = state
            .production_preparation_request()
            .expect("request with Enter-time Rotom selection");
        let current_prepared =
            AppState::calculate_production_preparation(current_request.clone(), cache)
                .expect("current production timeline");

        assert!(state.apply_production_preparation(&current_request, current_prepared));

        assert!(!state.encounter_start_requested);
        assert!(state.is_countdown());
    }

    #[test]
    fn positive_timing_adjustment_makes_the_beep_earlier_without_changing_sfmt() {
        let mut state = AppState::default();
        state.fps = "50".into();
        state.candidates = vec![TimelineCandidate {
            start_position: 0,
            sfmt_position: 100,
            next_blink_ticks: Some(300),
            target_reachable: true,
        }];
        state.next_blink_index = Some(100);
        let before = Instant::now() + Duration::from_secs(10);
        state.next_blink = Some(before);

        state.adjust_by(2);

        assert_eq!(state.timeline_display_position(), Some(100));
        assert_eq!(state.selected_candidate_position(), Some(100));
        let shift = before
            .duration_since(state.next_blink.expect("adjusted deadline"))
            .as_secs_f64();
        assert!((shift - 2.0 / 50.0).abs() < 0.001);
    }

    #[test]
    fn negative_timing_adjustment_makes_the_beep_later() {
        let mut state = AppState::default();
        state.fps = "50".into();
        let before = Instant::now() + Duration::from_secs(10);
        state.next_blink = Some(before);

        state.adjust_by(-2);

        let shift = state
            .next_blink
            .expect("adjusted deadline")
            .duration_since(before)
            .as_secs_f64();
        assert!((shift - 2.0 / 50.0).abs() < 0.001);
    }

    #[test]
    fn cancelling_observation_resets_ready_state_for_retry() {
        let mut state = AppState::default();
        state.mode = Mode::Ready;
        state.match_tolerance = 15;
        state.candidates.push(TimelineCandidate {
            start_position: 0,
            sfmt_position: 100,
            next_blink_ticks: Some(300),
            target_reachable: true,
        });
        state.cancel_observation();
        assert!(matches!(state.mode, Mode::Idle));
        assert!(state.candidates.is_empty());
        assert_eq!(state.match_tolerance, state.tolerance_value());
    }

    #[test]
    fn encounter_offset_uses_configured_timeline_fps() {
        let mut state = AppState::default();
        state.fps = "50".into();
        state.use_offset = true;
        state.encounter_offset = "100".into();
        assert!((state.encounter_offset_seconds() - 100.0 / 50.0).abs() < f64::EPSILON);

        state.encounter_offset = "101".into();
        assert!(state.encounter_offset_error().is_some());
    }

    #[test]
    fn production_time_steps_use_configured_timeline_fps() {
        let mut state = AppState::default();
        state.fps = "59.8702".into();
        assert!((state.game_ticks_to_seconds(30) - 60.0 / 59.8702).abs() < f64::EPSILON);
    }

    #[test]
    fn fps_value_accepts_machine_specific_calibration_and_falls_back_for_invalid_input() {
        let mut state = AppState::default();
        state.fps = "59.8702".into();
        assert_eq!(state.fps_value(), 59.8702);
        assert_eq!(state.search_config().unwrap().fps, 59.8702);

        state.fps = "0".into();
        assert_eq!(state.fps_value(), DEFAULT_FPS);

        state.fps = "not-a-number".into();
        assert_eq!(state.fps_value(), DEFAULT_FPS);
    }

    #[test]
    fn normal_timer_rejects_negative_total() {
        let mut timer = NormalTimerState::default();
        timer.wait_frames = "10".into();
        timer.offsets = vec!["-11".into()];
        assert!(timer.total_frames().is_err());
    }
}
