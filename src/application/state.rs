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
use crate::domain::rng::{encounter_plan_with_observed_rotom, rotom_roll_at};
use crate::domain::rng::{
    timeline_target_from_cache, ObservationCache, ProductionTimeline, ProductionTimelineCache,
};
use crate::errors::{AppError, InputField};
use std::collections::VecDeque;
use std::time::Instant;

pub(crate) use super::state_types::*;

mod correction;
mod encounter;
mod guidance;
mod observation;
mod production;
mod timer;

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

fn offset_after_observed_timeline_drift(
    current_offset: i64,
    actual_minus_target_frames: i64,
) -> i64 {
    current_offset.saturating_sub(actual_minus_target_frames)
}

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
    /// 正確なTarget待ちfallbackの最近の完了結果。計算自体はUI外のworkerで行う。
    normal_target_wait_cache: VecDeque<(NormalTargetWaitKey, Option<i64>)>,
    /// 候補集合が変わった時だけ進むrevision。UIはこの値で軽量keyを作る。
    candidate_generation: u64,
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
            normal_target_wait_cache: VecDeque::new(),
            candidate_generation: 0,
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
mod tests;
