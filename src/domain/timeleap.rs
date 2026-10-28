//! Timeleap候補の生成とFrame1後の候補照合。
//!
//! 指定された現在地からの経過時間範囲を通常Timelineで進め、候補ごとに
//! 状態を複製してイベントを適用し、Targetへ厳密に着地できる候補だけを残す。
//! このモジュールはその状態遷移をUIから分離する。

#[cfg(test)]
use super::rng::ModelConsumption;
use super::rng::ModelStatus;

mod candidate_search;
mod confirmed_search;
mod runtime;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use candidate_search::{
    candidates_have_same_future_state, match_shift_times, matching_shift_continuations,
    matching_shift_continuations_cancelable, search_candidates_from_cache,
    search_candidates_from_cache_cancelable, select_surviving_frame1, TimeleapShiftMatch,
};
#[allow(unused_imports)]
pub(crate) use candidate_search::{
    run_timeleap_candidate_search, run_timeleap_frame1_preparation, select_candidate,
    timeleap_target_reachable_after_frame1, visit_matching_shift_continuations_cancelable,
    TimeleapCandidate, TimeleapCandidateGroup, TimeleapContinuation, TimeleapFrame1Preparation,
    TimeleapSelection, TimeleapStatus, TimeleapTargetInput,
};
pub(crate) use confirmed_search::{
    confirmed_target_marker_boundary, run_confirmed_target_search, run_timeleap_reachability_check,
};
#[cfg(test)]
pub(crate) use runtime::predict_shift_times_until;
#[allow(unused_imports)]
pub(crate) use runtime::{advance_timeleap_runtime_tick, predict_shift_positions};

const MAX_CANDIDATES: usize = 100_000;
// One-model timelines can require substantially more game ticks than SFMT frames.
// This ceiling leaves headroom for the full 50-million-frame domain while still
// bounding malformed or unexpectedly stalled simulations.
const MAX_NORMAL_TIMELINE_TICKS: usize = 100_000_000;
// Early Frame1 candidates can need nearly the full target distance after the
// event. Their replay must use the same guard as the baseline timeline.
const MAX_CANDIDATE_REPLAY_TICKS: usize = 100_000_000;
const MERGE_CHECK_SFMT_INTERVAL: i64 = 64;
#[cfg(test)]
const MAX_CONSIDER_DELAY_SCAN_PATH_STATES: usize = 8_192;
const MAX_PARALLEL_TIMELEAP_WORKERS: usize = 4;
const MAX_TIMELEAP_MEMO_CACHE_BYTES: usize = 1024 * 1024 * 1024;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelRngEventKind {
    EnteredMinusFive {
        remain_after: i32,
    },
    ResolvedMinusFive {
        modulo_three: u64,
        remain_after: i32,
    },
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModelRngEvent {
    tick: i64,
    model: usize,
    sfmt_position: i64,
    kind: ModelRngEventKind,
}

#[cfg(test)]
thread_local! {
    static MODEL_RNG_TRACE: std::cell::RefCell<Option<Vec<ModelRngEvent>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn record_model_rng_events(tick: i64, consumed: &[ModelConsumption], remain: &[i32]) {
    for draw in consumed {
        record_model_rng_event(tick, *draw, remain[draw.model]);
    }
}

#[cfg(test)]
fn record_model_rng_event(tick: i64, draw: ModelConsumption, remain_after: i32) {
    let kind = if draw.blink {
        Some(ModelRngEventKind::ResolvedMinusFive {
            modulo_three: draw.value % 3,
            remain_after,
        })
    } else if draw.value & 0x7f == 0 {
        Some(ModelRngEventKind::EnteredMinusFive { remain_after })
    } else {
        None
    };
    let Some(kind) = kind else {
        return;
    };
    MODEL_RNG_TRACE.with(|trace| {
        if let Some(events) = trace.borrow_mut().as_mut() {
            events.push(ModelRngEvent {
                tick,
                model: draw.model,
                sfmt_position: draw.position,
                kind,
            });
        }
    });
}

#[cfg(test)]
fn begin_model_rng_trace() {
    MODEL_RNG_TRACE.with(|trace| {
        let mut trace = trace.borrow_mut();
        if trace.is_none() {
            *trace = Some(Vec::new());
        }
    });
}

#[cfg(test)]
fn finish_model_rng_trace() -> Vec<ModelRngEvent> {
    MODEL_RNG_TRACE.with(|trace| trace.borrow_mut().take().unwrap_or_default())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TimeleapMode {
    #[default]
    Menu,
    Dialogue,
    Courier,
}

impl TimeleapMode {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Menu => 0,
            Self::Dialogue => 1,
            Self::Courier => 2,
        }
    }
}

impl TryFrom<u8> for TimeleapMode {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Menu),
            1 => Ok(Self::Dialogue),
            2 => Ok(Self::Courier),
            other => Err(other),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CourierConfig {
    /// Event7 Consider Delay generates the Wonder Card after the first two ticks.
    pub no_dex: bool,
    /// 3DSRNGTool Event7 also generates once during delay for Your ID, unless
    /// the selected event is an egg.
    pub your_id: bool,
    pub is_egg: bool,
    pub ec: Option<u32>,
    pub pid_type: u8,
    pub pid: u32,
    pub iv_count: u8,
    pub fixed_ivs: [i8; 6],
    pub ability_locked: bool,
    pub ability: u8,
    pub nature_locked: bool,
    pub nature: u8,
    pub gender_locked: bool,
    pub gender_ratio: u8,
}

/// Immutable generation inputs consumed by the Timeleap transition itself.
/// Event7 Consider Delay flags intentionally live only on `CourierConfig` and
/// are not representable at this transition boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CourierGenerationProfile {
    pub(crate) ec: Option<u32>,
    pub(crate) pid_type: u8,
    pub(crate) pid: u32,
    pub(crate) iv_count: u8,
    pub(crate) fixed_ivs: [i8; 6],
    pub(crate) ability_locked: bool,
    pub(crate) ability: u8,
    pub(crate) nature_locked: bool,
    pub(crate) nature: u8,
    pub(crate) gender_locked: bool,
    pub(crate) gender_ratio: u8,
}

impl Default for CourierConfig {
    fn default() -> Self {
        Self {
            no_dex: false,
            your_id: false,
            is_egg: false,
            ec: None,
            pid_type: 0,
            pid: 0,
            iv_count: 0,
            fixed_ivs: [-1; 6],
            ability_locked: false,
            ability: 0,
            nature_locked: false,
            nature: 0,
            gender_locked: false,
            gender_ratio: 127,
        }
    }
}

impl CourierConfig {
    pub(crate) fn generation_profile(&self) -> CourierGenerationProfile {
        CourierGenerationProfile {
            ec: self.ec,
            pid_type: self.pid_type,
            pid: self.pid,
            iv_count: self.iv_count,
            fixed_ivs: self.fixed_ivs,
            ability_locked: self.ability_locked,
            ability: self.ability,
            nature_locked: self.nature_locked,
            nature: self.nature,
            gender_locked: self.gender_locked,
            gender_ratio: self.gender_ratio,
        }
    }

    fn generates_during_event_delay(&self) -> bool {
        self.no_dex || (self.your_id && !self.is_egg)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapConfig {
    pub seed: u32,
    pub start_frame: i64,
    pub target_frame: i64,
    pub models: usize,
    /// 現在位置からFrame1候補までの経過時間。1/30秒tick単位。
    pub min_frame1_elapsed_ticks: i64,
    pub max_frame1_elapsed_ticks: i64,
    pub mode: TimeleapMode,
    pub is_boy: bool,
    /// UI range in display Frames; runtime converts this to half as many Timeline ticks.
    pub handoff_range_frames: i64,
    pub courier: CourierConfig,
}

/// 確定済み通常Timelineのカーソル位置とモデル状態。
/// 任意のSFMT位置でremainを初期化し直さず、実際のTimelineをFrame1まで継続する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapTimelineStart {
    pub frame: i64,
    pub sfmt_position: i64,
    pub tick: i64,
    pub model: ModelStatus,
}

impl TimeleapTimelineStart {
    pub(crate) fn initial(frame: i64, models: usize) -> Self {
        Self {
            frame,
            sfmt_position: frame,
            tick: 0,
            model: ModelStatus::new(models),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapSearchKey {
    pub config: TimeleapConfig,
    /// Consider Delay is an encounter-generator transition, not the selected
    /// Timeleap trigger. The selected Fixed RNG entry determines the profile.
    pub consider_delay: Option<ConsiderDelayConfig>,
    pub grace_min: i64,
    pub target_grace_min: i64,
    pub frame1_filter: Option<(i64, i64)>,
    /// Confirmed pre-event Timeline state at the last observed blink.
    pub timeline_start: TimeleapTimelineStart,
    /// Absolute Timeline tick represented by `config.start_frame` when Search was pressed.
    pub current_tick: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsiderDelayGenerator {
    FixedSymbol(crate::domain::fixed_rng_catalog::StationaryDelayType),
    Event,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConsiderDelayConfig {
    /// Raw 3DSRNGTool delay value in display Frames. DelayTime is value / 2 + 2.
    pub delay_frames: i64,
    pub generator: ConsiderDelayGenerator,
    pub courier: CourierConfig,
}

#[derive(Debug)]
pub(crate) enum TimeleapSearchFailure {
    Cancelled,
    Frame1Unreachable,
    GenerationFrameUnreachable,
    Cache(crate::errors::AppError),
    Calculation(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TimeleapReachability {
    Frame1Unreachable,
    GenerationFrameUnreachable,
    Reachable { target_input_frames: Vec<i64> },
}

/// Explicitly entered values for the standalone reachability check.
/// The Timeline is reconstructed from this start position and the SFMT cache;
/// no active observation candidate or running timer state is reused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapReachabilityRequest {
    pub config: TimeleapConfig,
    pub frame1: i64,
    pub consider_delay: Option<ConsiderDelayConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedTimeleapStart {
    pub frame: i64,
    pub sfmt_position: i64,
    pub tick: i64,
    pub status: TimeleapStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedTargetSearchRequest {
    pub seed: u32,
    pub confirmed_frame1: i64,
    pub timeline_generation: u64,
    /// When present, rebuild the pre-Frame1 Timeline from the editable search source.
    pub source_config: Option<TimeleapConfig>,
    pub start: ConfirmedTimeleapStart,
    pub baseline_target_frame: i64,
    pub generation_start_frame: i64,
    pub drift_search_generation_start_frame: Option<i64>,
    pub drift_range_ticks: i64,
    pub grace_min_frames: i64,
    pub consider_delay: Option<ConsiderDelayConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedTargetSearchRow {
    pub frame1: i64,
    pub target_frame: i64,
    pub drift_frames: i64,
    pub grace_frames: i64,
    pub generation_start_frame: Option<i64>,
}

/// Complete search output with display-only metadata prepared away from the UI thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedTargetSearchResult {
    pub rows: Vec<ConfirmedTargetSearchRow>,
    pub drift_marker_boundary: Option<usize>,
}

impl ConfirmedTargetSearchResult {
    pub(crate) fn from_rows(
        rows: Vec<ConfirmedTargetSearchRow>,
        drift_search_generation_start_frame: Option<i64>,
    ) -> Self {
        let drift_marker_boundary = drift_search_generation_start_frame
            .and_then(|frame| confirmed_target_marker_boundary(&rows, frame));
        Self {
            rows,
            drift_marker_boundary,
        }
    }
}
