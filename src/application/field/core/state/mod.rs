//! フィールド観測のセッション状態と検索結果を管理する。
//!
//! UIから受け取った観測値を検索入力へ変換し、候補の確定、補正、
//! 本番タイマーへの引き渡しに必要な状態を一つのセッションとして保持する。

use super::*;
mod observation;
mod timeline;
mod timer;
#[cfg(test)]
use timeline::trim_timeline_history;

#[path = "timeleap/mod.rs"]
mod timeleap;
use crate::application::defaults;
use crate::application::state_types::SessionPhase;
use crate::domain::rng::{CacheReadError, ObservationCache, MAX_SFMT_FRAME};
#[cfg(test)]
use crate::domain::timeleap::matching_shift_continuations;
#[cfg(test)]
use crate::domain::timeleap::predict_shift_times_until;
#[cfg(test)]
use crate::domain::timeleap::search_candidates_from_cache;
use crate::domain::timeleap::{
    advance_timeleap_runtime_tick, select_candidate, CourierConfig, TimeleapCandidate,
    TimeleapCandidateGroup, TimeleapConfig, TimeleapContinuation, TimeleapMode, TimeleapSearchKey,
    TimeleapStatus,
};
use crate::errors::{AppError, AppResult, CacheError, InputField, StateIssue};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RESOLVED_TIMELINE_CHECKPOINT_TICKS: usize = 4_096;
const RESOLVED_TIMELINE_LOCK_BATCH_TICKS: usize = 256;
const DEFAULT_TIMELEAP_RECHECK_POSITION_RADIUS: i64 = 100;

pub(crate) fn timeleap_frame1_offset_drift(candidate_tick: i64, selected_tick: i64) -> Option<i64> {
    let max_tick = crate::domain::rng::MAX_SFMT_FRAME;
    if !(0..=max_tick).contains(&candidate_tick) || !(0..=max_tick).contains(&selected_tick) {
        return None;
    }
    candidate_tick.checked_sub(selected_tick)?.checked_mul(2)
}

fn timeleap_offset_after_drift(current: i64, drift_frames: i64) -> Option<i64> {
    let limit = crate::domain::rng::MAX_SFMT_FRAME;
    if !(-limit..=limit).contains(&current) || current % 2 != 0 || drift_frames % 2 != 0 {
        return None;
    }
    // drift_frames = actual/candidate position - desired position. Move the
    // requested Offset in the opposite direction, independent of close mode.
    let corrected = current.checked_sub(drift_frames)?;
    ((-limit..=limit).contains(&corrected) && corrected % 2 == 0).then_some(corrected)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeleapMethodSelection {
    Dialogue,
    MenuBoy,
    MenuGirl,
    Courier,
}

impl TimeleapMethodSelection {
    pub(crate) fn from_state(mode: TimeleapMode, is_boy: bool) -> Self {
        match mode {
            TimeleapMode::Dialogue => Self::Dialogue,
            TimeleapMode::Courier => Self::Courier,
            TimeleapMode::Menu if is_boy => Self::MenuBoy,
            TimeleapMode::Menu => Self::MenuGirl,
        }
    }

    pub(crate) fn state_values(self) -> (TimeleapMode, bool) {
        match self {
            Self::Dialogue => (TimeleapMode::Dialogue, true),
            Self::MenuBoy => (TimeleapMode::Menu, true),
            Self::MenuGirl => (TimeleapMode::Menu, false),
            Self::Courier => (TimeleapMode::Courier, true),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimelineBlinkEvent {
    tick: u32,
    consumption: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineCheckpoint {
    generated_ticks: usize,
    consumption: i64,
    cursor: TimelineCursor,
}

/// Session-scoped append-only progression shared by Current, table extension and Target queries.
/// The tick deltas are compact; blink events and sparse checkpoints provide indexed lookups.
struct ResolvedTimelineCache {
    identity: TimelineIdentity,
    cursor: TimelineCursor,
    sfmt: Sfmt,
    selected_model: usize,
    anchor_tick: i64,
    anchor_consumption: i64,
    tick_deltas: Vec<u8>,
    checkpoints: Vec<TimelineCheckpoint>,
    blinks: Vec<TimelineBlinkEvent>,
    /// First post-Timeleap point whose full remain vector is in the SafeFrame state.
    safe_frame: Option<i64>,
}

fn is_safe_frame_cursor(cursor: &TimelineCursor) -> bool {
    cursor.timeleap_runtime.is_some()
        && !cursor.status.remain.is_empty()
        && cursor
            .status
            .remain
            .iter()
            .all(|remain| matches!(*remain, 0 | 1))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineIdentity {
    seed: u32,
    start_consumption: i64,
    first_wait_frame: i64,
    last_wait_frame: i64,
    first_consumption: i64,
    last_consumption: i64,
    models: usize,
    observed_model: usize,
    intervals: Vec<i64>,
    target_consumptions: Vec<i64>,
}

/// Minimal immutable work package for extending the visible field timeline.
/// It deliberately contains neither the full FieldState nor candidate blink history.
pub(crate) struct TimelineExtensionRequest {
    identity: TimelineIdentity,
    timeleap_mode: TimeleapMode,
    result: FieldResult,
    observation_cache: Option<ObservationCache>,
    resolved_timeline_cache: Option<Arc<Mutex<ResolvedTimelineCache>>>,
    projection_through_tick: i64,
    timeleap_stage: TimeleapStage,
    timeleap_timeline_anchor: Option<Instant>,
    timeleap_recheck_pause_frame: Option<i64>,
    observation_anchor: Option<Instant>,
    frame1_target_consumption: Option<i64>,
}

/// Timeline-only result. The UI applies it only while its generation and identity match.
pub(crate) struct TimelineExtensionResult {
    identity: TimelineIdentity,
    timeleap_mode: TimeleapMode,
    result: FieldResult,
    resolved_timeline_cache: Option<Arc<Mutex<ResolvedTimelineCache>>>,
    projection_through_tick: i64,
    safe_frame: Option<i64>,
    frame1_table_boundary: Option<(i64, usize)>,
    can_continue: bool,
}

/// Compact result after the worker classifies matching Shift continuations.
/// It avoids sending a potentially large match list back for UI-thread scanning.
#[derive(Default)]
pub(crate) struct TimeleapShiftResolutionSummary {
    matches_found: bool,
    multiple_future_states: bool,
    confirmed_frame1: Option<i64>,
    confirmed_candidate: Option<TimeleapCandidate>,
}

impl TimeleapShiftResolutionSummary {
    pub(crate) fn has_no_matches(&self) -> bool {
        !self.matches_found && !self.multiple_future_states
    }
}

fn advance_timeline_cursor(cursor: &mut TimelineCursor, sfmt: &mut Sfmt) -> (i64, Vec<u8>) {
    let (used, blinks) = if let Some(runtime) = &mut cursor.timeleap_runtime {
        advance_timeleap_runtime_tick(&mut cursor.status, runtime, &mut cursor.stream_cursor, sfmt)
    } else {
        let (used, consumed) = cursor.status.next_state_with_positions(|| {
            let position = cursor.stream_cursor;
            let value = sfmt.next_u64();
            cursor.stream_cursor = cursor.stream_cursor.saturating_add(1);
            (position, value)
        });
        (
            used as i64,
            consumed
                .into_iter()
                .filter_map(|event| event.blink.then_some(event.model as u8))
                .collect(),
        )
    };
    cursor.consumption = cursor.consumption.saturating_add(used);
    cursor.tick = cursor.tick.saturating_add(1);
    (used, blinks)
}

impl ResolvedTimelineCache {
    fn new(
        identity: TimelineIdentity,
        cursor: TimelineCursor,
        sfmt: Sfmt,
        selected_model: usize,
    ) -> Self {
        let anchor_tick = cursor.tick;
        let anchor_consumption = cursor.consumption;
        let safe_frame = is_safe_frame_cursor(&cursor).then_some(anchor_consumption);
        let initial_cursor = cursor.clone();
        Self {
            identity,
            cursor,
            sfmt,
            selected_model,
            anchor_tick,
            anchor_consumption,
            tick_deltas: Vec::new(),
            checkpoints: vec![TimelineCheckpoint {
                generated_ticks: 0,
                consumption: anchor_consumption,
                cursor: initial_cursor,
            }],
            blinks: Vec::new(),
            safe_frame,
        }
    }

    #[cfg(test)]
    fn append_chunk(&mut self, max_ticks: usize) -> usize {
        self.append_chunk_cancelable(max_ticks, None)
    }

    fn append_chunk_cancelable(
        &mut self,
        max_ticks: usize,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> usize {
        let initial = self.tick_deltas.len();
        let maximum_tick = MAX_FUTURE_TIMELINE_TICKS;
        let maximum_consumption = crate::domain::rng::MAX_SFMT_FRAME;
        while self.tick_deltas.len().saturating_sub(initial) < max_ticks
            && self.cursor.tick < maximum_tick
            && self.cursor.consumption < maximum_consumption
        {
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                break;
            }
            let (used, blinks) = advance_timeline_cursor(&mut self.cursor, &mut self.sfmt);
            self.tick_deltas.push(used as u8);
            if self.safe_frame.is_none() && is_safe_frame_cursor(&self.cursor) {
                self.safe_frame = Some(self.cursor.consumption);
            }
            if blinks.contains(&(self.selected_model as u8)) {
                self.blinks.push(TimelineBlinkEvent {
                    tick: self.cursor.tick as u32,
                    consumption: self.cursor.consumption,
                });
            }
            let generated = self.tick_deltas.len();
            if generated % RESOLVED_TIMELINE_CHECKPOINT_TICKS == 0 {
                self.checkpoints.push(TimelineCheckpoint {
                    generated_ticks: generated,
                    consumption: self.cursor.consumption,
                    cursor: self.cursor.clone(),
                });
            }
        }
        self.tick_deltas.len().saturating_sub(initial)
    }

    fn consumption_at_tick(&self, tick: i64) -> i64 {
        let generated_tick = tick.saturating_sub(self.anchor_tick).max(0) as usize;
        let generated_tick = generated_tick.min(self.tick_deltas.len());
        let checkpoint_index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.generated_ticks <= generated_tick)
            .saturating_sub(1);
        let checkpoint = &self.checkpoints[checkpoint_index];
        self.tick_deltas[checkpoint.generated_ticks..generated_tick]
            .iter()
            .fold(checkpoint.consumption, |sum, delta| sum + i64::from(*delta))
    }

    fn cursor_at_tick(
        &self,
        tick: i64,
        observation_cache: &ObservationCache,
    ) -> Result<Option<TimelineCursor>, AppError> {
        if tick < self.anchor_tick || tick > self.cursor.tick {
            return Ok(None);
        }
        if tick == self.cursor.tick {
            return Ok(Some(self.cursor.clone()));
        }
        let generated_tick = tick.saturating_sub(self.anchor_tick) as usize;
        let checkpoint_index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.generated_ticks <= generated_tick)
            .saturating_sub(1);
        let mut cursor = self.checkpoints[checkpoint_index].cursor.clone();
        let mut sfmt = observation_cache
            .sfmt_at_checked(cursor.stream_cursor)?
            .ok_or(AppError::Cache(CacheError::StateUnavailable))?;
        while cursor.tick < tick {
            advance_timeline_cursor(&mut cursor, &mut sfmt);
        }
        Ok(Some(cursor))
    }

    fn target_timing(&self, target: i64) -> Option<(i64, bool, i64, i64, usize)> {
        if target < self.anchor_consumption {
            return None;
        }
        if target > self.cursor.consumption {
            return None;
        }
        if target == self.anchor_consumption {
            return Some((self.anchor_tick * 2, true, target, target, 0));
        }
        let checkpoint_index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.consumption < target)
            .saturating_sub(1);
        let checkpoint = &self.checkpoints[checkpoint_index];
        let mut consumption = checkpoint.consumption;
        let mut blink_index = self.blinks.partition_point(|event| {
            i64::from(event.tick) <= self.anchor_tick + checkpoint.generated_ticks as i64
        });
        let mut blink_count = blink_index;
        let end = self.tick_deltas.len();
        for index in checkpoint.generated_ticks..end {
            let before = consumption;
            consumption += i64::from(self.tick_deltas[index]);
            let tick = self.anchor_tick + index as i64 + 1;
            if before <= target && target < consumption {
                return Some((tick * 2, false, before, consumption, blink_count));
            }
            if consumption == target {
                return Some((tick * 2, true, target, target, blink_count));
            }
            if self
                .blinks
                .get(blink_index)
                .is_some_and(|event| i64::from(event.tick) == tick)
            {
                blink_index += 1;
                blink_count += 1;
            }
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
// ---------------------------------------------------------------------------
// フィールドタブのセッション状態とUI向け遷移
// ---------------------------------------------------------------------------

pub enum FieldMode {
    Idle,
    Observing,
    Correction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldTimerKind {
    NextBlink,
    Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeleapStage {
    Off,
    Frame1Waiting,
    Assumed,
    ShiftWaiting,
    Failed,
    Succeeded,
    Recheck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeleapLifecyclePhase {
    Disabled,
    Frame1Setup,
    Frame1Preparing,
    Frame1Waiting,
    Frame1Unreachable,
    Frame1Failed,
    PostTimeleapAssumed,
    PostTimeleapRechecking,
    PostTimeleapConfirmed,
    PostTimeleapRecheckFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeleapTableMode {
    Target,
    Frame1,
}

/// One projection of the Timeleap lifecycle, shared by the table, summary,
/// and background timeline-extension request. Timer selection is orthogonal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapLifecyclePolicy {
    pub phase: TimeleapLifecyclePhase,
    pub table_mode: TimeleapTableMode,
    pub frame1_boundary_target: Option<i64>,
    pub summary_frame1: Option<i64>,
    pub configuration_locked: bool,
    pub frame1_input_enabled: bool,
    pub candidate_search_enabled: bool,
    pub frame1_has_passed: bool,
}

/// Immutable, validated snapshot for rechecking observed Timeleap blinks.
pub(crate) struct TimeleapShiftResolutionRequest {
    pub generation: u64,
    cache: ObservationCache,
    candidates: Vec<TimeleapCandidate>,
    model: usize,
    observed: Vec<i64>,
    correction: i64,
    tolerance: i64,
    until_frame: i64,
    nominal_tick: i64,
    recheck_range_frames: i64,
    selected_frame1: Option<i64>,
}

impl TimeleapShiftResolutionRequest {
    pub(crate) fn resolve(
        self,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Option<TimeleapShiftResolutionSummary> {
        let mut has_matches = false;
        let mut multiple_future_states = false;
        let mut unique_future_state: Option<(
            i64,
            i64,
            i64,
            crate::domain::timeleap::TimeleapStatus,
        )> = None;
        let mut frame1_candidates =
            std::collections::BTreeMap::<i64, (usize, TimeleapCandidate)>::new();
        for candidate in self.candidates.iter().filter(|candidate| {
            frame1_tick_is_in_research_range(
                candidate.frame1_tick,
                self.nominal_tick,
                self.recheck_range_frames,
            )
        }) {
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                return None;
            }
            crate::domain::timeleap::visit_matching_shift_continuations_cancelable(
                &self.cache,
                candidate,
                self.model,
                &self.observed,
                self.correction,
                self.tolerance,
                self.until_frame,
                cancel,
                |continuation| {
                    has_matches = true;
                    let next_state = (
                        continuation.post_frame,
                        continuation.post_sfmt_position,
                        continuation.post_tick,
                        continuation.post_status.clone(),
                    );
                    if unique_future_state
                        .as_ref()
                        .is_some_and(|existing| existing != &next_state)
                    {
                        multiple_future_states = true;
                    } else {
                        unique_future_state.get_or_insert(next_state);
                    }
                    frame1_candidates
                        .entry(continuation.frame1)
                        .and_modify(|(count, _)| *count += 1)
                        .or_insert((1, continuation));
                    true
                },
            )?;
        }
        if !has_matches || multiple_future_states {
            return Some(TimeleapShiftResolutionSummary {
                matches_found: has_matches,
                multiple_future_states,
                ..TimeleapShiftResolutionSummary::default()
            });
        }

        let confirmed_frame1 = self
            .selected_frame1
            .filter(|frame| frame1_candidates.contains_key(frame))
            .or_else(|| {
                let middle = frame1_candidates
                    .values()
                    .map(|(count, _)| count)
                    .sum::<usize>()
                    / 2;
                let mut skipped = 0usize;
                frame1_candidates.iter().find_map(|(frame, (count, _))| {
                    if middle < skipped.saturating_add(*count) {
                        Some(*frame)
                    } else {
                        skipped = skipped.saturating_add(*count);
                        None
                    }
                })
            });
        let confirmed_candidate = confirmed_frame1
            .and_then(|frame| frame1_candidates.remove(&frame))
            .map(|(_, candidate)| candidate);
        Some(TimeleapShiftResolutionSummary {
            matches_found: true,
            multiple_future_states: false,
            confirmed_frame1,
            confirmed_candidate,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TimeleapSearchStatus {
    #[default]
    NotRequested,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeleapFrame1Indicator {
    Reachable,
    Unreachable,
}

#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // UIタブを外した後も独立到達計算の入力モデルを保持する。
pub(crate) struct TimeleapReachabilityForm {
    pub initialized: bool,
    pub seed: String,
    /// Total model count, including the player model.
    pub models: usize,
    pub game_edition: crate::domain::fixed_rng_catalog::GameEdition,
    pub category: usize,
    pub preset: usize,
    pub mode: TimeleapMode,
    pub is_boy: bool,
    pub starting_frame: String,
    pub frame1: String,
    pub consider_delay_enabled: bool,
    pub consider_delay_frames: String,
    pub generation_start_frame: String,
    pub courier: CourierConfig,
    pub status: TimeleapReachabilityStatus,
    pub generation: u64,
}

#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // 到達可否タブは非表示だが、独立計算ロジックは保持する。
pub(crate) enum TimeleapReachabilityStatus {
    #[default]
    Idle,
    Running,
    Frame1Unreachable,
    GenerationFrameUnreachable,
    Reachable(Vec<i64>),
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ConfirmedTargetSearchForm {
    pub initialized: bool,
    pub drift_search_enabled: bool,
    pub drift_search_generation_start_frame: String,
    pub drift_range_ticks: String,
    pub grace_min_frames: String,
    /// Form values captured by the last explicit search; result decoration must not drift
    /// while the user edits the next search.
    pub result_generation_start_frame: Option<i64>,
    pub result_drift_search_generation_start_frame: Option<i64>,
    /// Result-only marker location, computed once when rows are accepted.
    pub result_drift_marker_boundary: Option<usize>,
    pub generation: u64,
    pub status: ConfirmedTargetSearchStatus,
    /// 再検索中だけ退避する直近の完了状態。ArcによりFieldState複製時の行cloneを避ける。
    previous_status: Option<Arc<ConfirmedTargetSearchStatus>>,
    request: Option<crate::domain::timeleap::ConfirmedTargetSearchRequest>,
}

#[derive(Clone, Debug, Default)]
pub(crate) enum ConfirmedTargetSearchStatus {
    #[default]
    Idle,
    Running,
    NoResults,
    Frame1Unreachable,
    GenerationFrameUnreachable,
    Results(Vec<crate::domain::timeleap::ConfirmedTargetSearchRow>),
    Failed(String),
}

#[derive(Clone)]
pub struct FieldState {
    mode: FieldMode,
    /// 観測対象idxを指定するか、全idxを候補として扱うか。
    pub target_mode: FieldTargetMode,
    pub seed: String,
    pub range_start: String,
    pub range_end: String,
    /// 「start +」で検索終点を決めるための幅。
    pub range_after_start: String,
    /// タイムライン仕様のNPC数欄。主人公1体は`config`内で暗黙に加算する。
    pub npc_count: String,
    /// NPC欄を`始点~終点`として展開する検索モード。
    pub multiple_npc_search: bool,
    /// 初期読み込み境界（-5分岐）を基準位置から探す幅。
    pub initial_branch_scan_span: String,
    /// Timeleapを有効にするか。
    pub timeleap_enabled: bool,
    /// 実際に乗り移りを発生させるFrame1。候補表はこの値を変更する補助UI。
    pub timeleap_frame1_input: String,
    pub timeleap_mode: TimeleapMode,
    pub timeleap_is_boy: bool,
    pub timeleap_default_is_boy: bool,
    pub timeleap_search_range: String,
    pub timeleap_seconds_min: String,
    pub timeleap_seconds_max: String,
    pub timeleap_frame1_min: String,
    pub timeleap_frame1_max: String,
    /// 0=seconds, 1=absolute Frame1 range for candidate filtering.
    pub timeleap_filter_mode: u8,
    pub timeleap_grace_min: String,
    pub timeleap_target_grace_min: String,
    pub timeleap_generation_start_frame: String,
    /// 3DSRNGToolのイベント個体生成を遅延内で行う設定。配達員経路だけで有効。
    pub timeleap_no_dex: bool,
    /// Timeleap採用後に瞬きでなくTargetFrameの主タイマーを選択する。
    pub timeleap_direct_encounter: bool,
    pub timeleap_consider_delay_enabled: bool,
    pub timeleap_consider_delay_frames: String,
    pub timeleap_game_edition: crate::domain::fixed_rng_catalog::GameEdition,
    pub timeleap_search_category: usize,
    pub timeleap_search_preset: usize,
    pub timeleap_search_tab: u8,
    /// Timeleap search-window source inputs shared by Main and TargetFrame search.
    pub timeleap_source_inputs_initialized: bool,
    pub timeleap_source_seed: String,
    pub timeleap_source_starting_frame: String,
    pub timeleap_source_npc_count: String,
    pub timeleap_source_frame1: String,
    /// 候補検索が利用者から一度明示的に要求されたか。
    /// 一度検索した後は、条件変更時にボタン表記を「再検索」へ切り替える。
    pub(crate) timeleap_candidate_search_started: bool,
    pub timeleap_courier_iv_count: String,
    pub timeleap_courier_ability_locked: bool,
    pub timeleap_courier_nature_locked: bool,
    pub timeleap_courier_gender_locked: bool,
    pub timeleap_courier_ec: String,
    pub timeleap_courier_pid_type: u8,
    pub timeleap_courier_pid: String,
    pub timeleap_courier_fixed_ivs: [i8; 6],
    pub timeleap_courier_ability: u8,
    pub timeleap_courier_nature: u8,
    pub timeleap_courier_gender_ratio: u8,
    pub timeleap_courier_is_egg: bool,
    pub timeleap_courier_your_id: bool,
    pub timeleap_courier_other_info: bool,
    pub timeleap_courier_tid: String,
    pub timeleap_courier_sid: String,
    /// 表示用の種族番号。乱数計算には使用しない。
    pub timeleap_courier_species_id: u16,
    pub timeleap_search_window_open: bool,
    pub(crate) timeleap_reachability_form: TimeleapReachabilityForm,
    pub(crate) confirmed_target_search_form: ConfirmedTargetSearchForm,
    pub(crate) confirmed_target_timeline_generation: u64,
    pub timeleap_candidate_groups: Vec<TimeleapCandidateGroup>,
    pub timeleap_frame1_preparing: bool,
    timeleap_execution_candidates: Vec<TimeleapCandidate>,
    timeleap_search_status: TimeleapSearchStatus,
    timeleap_candidate_search_key: Option<TimeleapSearchKey>,
    timeleap_candidate_search_started_at: Option<Instant>,
    pub timeleap_selected_frame: Option<i64>,
    pub timeleap_selected_sfmt_position: Option<i64>,
    pub timeleap_confirmed_frame1: Option<i64>,
    timeleap_confirmed_candidate: Option<TimeleapCandidate>,
    pub timeleap_locked: bool,
    pub timeleap_stage: TimeleapStage,
    pub timeleap_shift_times: Vec<Instant>,
    timeleap_shift_resolution_generation: u64,
    timeleap_shift_resolution_pending: bool,
    pub timeleap_frame1_target: Option<i64>,
    timeleap_nominal_tick: Option<i64>,
    timeleap_frame1_wait_seconds: f64,
    timeleap_frame1_deadline: Option<Instant>,
    timeleap_frame1_preview_state: TimeleapFrame1PreviewState,
    /// Frame1 blink count computed by the timeline worker when Frame1 is beyond displayed rows.
    timeleap_frame1_table_boundary: Option<(i64, usize)>,
    timeleap_target_check_pending: bool,
    timeleap_target_reachable: Option<bool>,
    /// Exact pre-event reachability result; None means pending, invalid, or unavailable.
    pub timeleap_frame1_reachable: Option<bool>,
    timeleap_frame1_preview_generation: u64,
    timeleap_wait_started: Option<Instant>,
    timeleap_observation_anchor_frame: Option<i64>,
    timeleap_timeline_anchor: Option<Instant>,
    /// Adjustment already consumed by the Frame1 deadline for a provisional
    /// post-Timeleap clock. A confirmed Shift adoption uses its measured anchor
    /// instead and leaves this unset.
    timeleap_frame1_adjustment_consumed: Option<i64>,
    /// Event-transition time from Frame1 to the post-Timeleap cursor, in game frames.
    /// Applied only while the Ready-provisionally-adopted clock is anchored at Frame1.
    timeleap_assumed_transition_frames: i64,
    timeleap_recheck_pause_frame: Option<i64>,
    /// Regular blink-interval tolerance temporarily replaced by the recheck position radius.
    timeleap_recheck_interval_tolerance: Option<String>,
    /// Immutable pre-Timeleap origin; subsequent Timeline adoption must not replace it.
    timeleap_original_starting_frame: Option<i64>,
    /// SafeFrame selected from the adopted Timeleap Timeline for 3DSRNGTool display only.
    pub(crate) timeleap_safe_frame_display: Option<i64>,
    /// 観測対象idx。0は主人公、1以上はNPC。
    pub target_model: String,
    /// 実際に取得できたTargetFrame。idx自動判定に使用する。
    pub actual_frame: String,
    pub tolerance: String,
    pub target_consumption: String,
    target_last_committed: String,
    /// Targetタイマーだけに適用する表示Frameオフセット。観測Timelineは変更しない。
    pub encounter_offset: String,
    pub use_offset: bool,
    /// TargetFrame欄を編集してから、Enterまたはフォーカス移動で再検索するための状態。
    pub target_dirty: bool,
    /// Targetだけを変更して、観測済みの候補を再評価している状態。
    /// この間も既存の瞬きタイマーは止めない。
    pub target_research_pending: bool,
    /// SFMT/Timeline上限まで進み、次の瞬きが範囲外になった状態。
    pub next_blink_out_of_range: bool,
    /// 明示的にEnterで補正へ進んだ後はTargetFrameを固定する。
    /// 観測結果確定直後からEnterまでは、瞬きタイマー実行中でも編集できる。
    target_input_locked: bool,
    pub observed: Vec<Instant>,
    pub intervals: Vec<i64>,
    pub results: Vec<FieldResult>,
    /// 指定範囲を照合して候補0件で終了した状態。追加のShiftでは再検索しない。
    pub search_exhausted: bool,
    /// 実測Frameから絞り込まれた観測対象idxの候補。
    pub inferred_indices: Vec<usize>,
    /// ずれ補正を最初に計算した結果を、同じ補正セッションで再利用する。
    pub correction_calculated: bool,
    pub correction_suggested_offset: Option<i64>,
    /// 指定idxで一致せず、別idxへフォールバックしたときの一致idx。
    pub idx_fallback_indices: Vec<usize>,
    /// idx指定時に、候補開始Frameのうち477以上で最も近いもの。
    pub nearest_timeline_frame: Option<i64>,
    pub selected_result: usize,
    timer_start: Option<Instant>,
    timer_seconds: f64,
    /// Targetタイマーが完了して表示を0へ固定している状態。
    timer_finished: bool,
    timer_kind: FieldTimerKind,
    /// ずれ検証へ入った瞬間のTimeline位置。補正中は観測アンカーの経過
    /// 時間ではなく、この位置を表のCurrentとして保持する。
    correction_timeline_frame: Option<i64>,
    /// 既に計算済みの次回以降の瞬き時刻。各瞬きのたびにTimelineを
    /// 再走査せず、観測タイマーを滑らかにつなぐために使う。
    blink_wait_queue: VecDeque<i64>,
    /// 右上表で前回スクロール追従した行。表示状態のリセットにも使う。
    pub table_last_followed_row: Option<usize>,
    /// 右上のTimeline表を現在位置へ自動追従させるか。初期値は有効。
    pub table_follow_current: bool,
    /// タイマーの音・表示位相を実機に合わせる表示Frame補正。
    pub adjust: i64,
    /// Field validation notices that are rendered in the observation panel.
    pub notice: Option<String>,
    observation_cache: Option<ObservationCache>,
    resolved_timeline_cache: Option<Arc<Mutex<ResolvedTimelineCache>>>,
    timeline_projection_through_tick: i64,
    last_current_sfmt_frame: Option<i64>,
    timeline_stream_error: Option<std::sync::Arc<AppError>>,
}

#[derive(Clone)]
struct TimeleapFrame1Preview {
    config: TimeleapConfig,
    frame1: i64,
    target_frame: Option<i64>,
    start: crate::domain::timeleap::TimeleapTimelineStart,
    preparation: crate::domain::timeleap::TimeleapFrame1Preparation,
}

#[derive(Clone, Default)]
enum TimeleapFrame1PreviewState {
    #[default]
    Idle,
    Pending,
    Ready(TimeleapFrame1Preview),
    Unreachable,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeleapFrame1PreviewReconciliation {
    Stable,
    Schedule,
    Restart,
}

impl TimeleapFrame1PreviewState {
    fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

impl Default for FieldState {
    fn default() -> Self {
        Self {
            mode: FieldMode::Idle,
            target_mode: FieldTargetMode::Known,
            seed: defaults::SEED.into(),
            range_start: defaults::FIELD_RANGE_START.into(),
            range_end: defaults::FIELD_RANGE_END.into(),
            range_after_start: defaults::FIELD_RANGE_AFTER_START.into(),
            npc_count: defaults::FIELD_NPC_COUNT.into(),
            multiple_npc_search: false,
            initial_branch_scan_span: defaults::FIELD_INITIAL_BRANCH_SCAN_SPAN.into(),
            timeleap_enabled: false,
            timeleap_frame1_input: defaults::FIELD_TIMELEAP_FRAME1.into(),
            timeleap_mode: TimeleapMode::Menu,
            timeleap_is_boy: true,
            timeleap_default_is_boy: true,
            timeleap_search_range: defaults::FIELD_TIMELEAP_SEARCH_RANGE.into(),
            timeleap_seconds_min: defaults::FIELD_TIMELEAP_SECONDS_MIN.into(),
            timeleap_seconds_max: defaults::FIELD_TIMELEAP_SECONDS_MAX.into(),
            timeleap_frame1_min: "0".into(),
            timeleap_frame1_max: crate::domain::rng::MAX_SFMT_FRAME.to_string(),
            timeleap_filter_mode: 0,
            timeleap_grace_min: defaults::FIELD_TIMELEAP_GRACE_MIN.into(),
            timeleap_target_grace_min: "0".into(),
            timeleap_generation_start_frame: "10000".into(),
            timeleap_no_dex: false,
            timeleap_direct_encounter: false,
            timeleap_consider_delay_enabled: true,
            timeleap_consider_delay_frames: "0".into(),
            timeleap_game_edition: crate::domain::fixed_rng_catalog::GameEdition::UltraSunUltraMoon,
            timeleap_search_category: 0,
            timeleap_search_preset: 0,
            timeleap_search_tab: 0,
            timeleap_source_inputs_initialized: false,
            timeleap_source_seed: String::new(),
            timeleap_source_starting_frame: String::new(),
            timeleap_source_npc_count: String::new(),
            timeleap_source_frame1: String::new(),
            timeleap_candidate_search_started: false,
            timeleap_courier_iv_count: "0".into(),
            timeleap_courier_ability_locked: false,
            timeleap_courier_nature_locked: false,
            timeleap_courier_gender_locked: false,
            timeleap_courier_ec: String::new(),
            timeleap_courier_pid_type: 0,
            timeleap_courier_pid: String::new(),
            timeleap_courier_fixed_ivs: [-1; 6],
            timeleap_courier_ability: 0,
            timeleap_courier_nature: 0,
            timeleap_courier_gender_ratio: 127,
            timeleap_courier_is_egg: false,
            timeleap_courier_your_id: false,
            timeleap_courier_other_info: true,
            timeleap_courier_tid: "0".into(),
            timeleap_courier_sid: "0".into(),
            timeleap_courier_species_id: 0,
            timeleap_search_window_open: false,
            timeleap_reachability_form: TimeleapReachabilityForm::default(),
            confirmed_target_search_form: ConfirmedTargetSearchForm::default(),
            confirmed_target_timeline_generation: 0,
            timeleap_candidate_groups: Vec::new(),
            timeleap_frame1_preparing: false,
            timeleap_execution_candidates: Vec::new(),
            timeleap_search_status: TimeleapSearchStatus::NotRequested,
            timeleap_candidate_search_key: None,
            timeleap_candidate_search_started_at: None,
            timeleap_selected_frame: None,
            timeleap_selected_sfmt_position: None,
            timeleap_confirmed_frame1: None,
            timeleap_confirmed_candidate: None,
            timeleap_locked: false,
            timeleap_stage: TimeleapStage::Off,
            timeleap_shift_times: Vec::new(),
            timeleap_shift_resolution_generation: 0,
            timeleap_shift_resolution_pending: false,
            timeleap_frame1_target: None,
            timeleap_nominal_tick: None,
            timeleap_frame1_wait_seconds: 0.0,
            timeleap_frame1_deadline: None,
            timeleap_frame1_preview_state: TimeleapFrame1PreviewState::Idle,
            timeleap_frame1_table_boundary: None,
            timeleap_target_check_pending: false,
            timeleap_target_reachable: None,
            timeleap_frame1_reachable: None,
            timeleap_frame1_preview_generation: 0,
            timeleap_wait_started: None,
            timeleap_observation_anchor_frame: None,
            timeleap_timeline_anchor: None,
            timeleap_frame1_adjustment_consumed: None,
            timeleap_assumed_transition_frames: 0,
            timeleap_recheck_pause_frame: None,
            timeleap_recheck_interval_tolerance: None,
            timeleap_original_starting_frame: None,
            timeleap_safe_frame_display: None,
            target_model: defaults::FIELD_TARGET_MODEL.into(),
            actual_frame: String::new(),
            tolerance: defaults::FIELD_TOLERANCE.into(),
            target_consumption: String::new(),
            target_last_committed: String::new(),
            encounter_offset: defaults::FIELD_ENCOUNTER_OFFSET.into(),
            use_offset: defaults::DEFAULT_USE_OFFSET,
            target_dirty: false,
            target_research_pending: false,
            next_blink_out_of_range: false,
            target_input_locked: false,
            observed: Vec::new(),
            intervals: Vec::new(),
            results: Vec::new(),
            search_exhausted: false,
            inferred_indices: Vec::new(),
            correction_calculated: false,
            correction_suggested_offset: None,
            idx_fallback_indices: Vec::new(),
            nearest_timeline_frame: None,
            selected_result: 0,
            timer_start: None,
            timer_seconds: 0.0,
            timer_finished: false,
            timer_kind: FieldTimerKind::NextBlink,
            correction_timeline_frame: None,
            blink_wait_queue: VecDeque::new(),
            table_last_followed_row: None,
            table_follow_current: defaults::DEFAULT_TABLE_FOLLOW_CURRENT,
            adjust: defaults::DEFAULT_ADJUST,
            notice: None,
            observation_cache: None,
            resolved_timeline_cache: None,
            timeline_projection_through_tick: 0,
            last_current_sfmt_frame: None,
            timeline_stream_error: None,
        }
    }
}

fn parse_seed(value: &str) -> Result<u32, String> {
    let trimmed = value
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    u32::from_str_radix(trimmed, 16)
        .or_else(|_| trimmed.parse::<u32>())
        .map_err(|_| "初期SEEDを確認してください。".into())
}

/// 保存済みの全モデル状態が同じかを判定する。
///
/// 観測対象の未来列だけが一致する候補は、他モデルの`remain`やSFMTカーソルが異なる
/// 可能性があるため統合しない。`TimelineCursor`が保存されていない合成テスト結果では、
/// 開始位置まで含めた完全重複だけを統合対象とする。
fn same_full_timeline_state(left: &FieldResult, right: &FieldResult) -> bool {
    let Some(left_cursor) = left
        .target_cursor
        .as_ref()
        .or(left.timeline_cursor.as_ref())
    else {
        return right.target_cursor.is_none()
            && right.timeline_cursor.is_none()
            && left.start_consumption == right.start_consumption;
    };
    let Some(right_cursor) = right
        .target_cursor
        .as_ref()
        .or(right.timeline_cursor.as_ref())
    else {
        return false;
    };
    // `tick`は各候補の観測開始からの相対経過時間であり、乱数状態ではない。
    // 開始位置が異なる候補は同じSFMTカーソルへ到達してもtickだけ異なるため、
    // remain・モデル状態・SFMTカーソル・消費数だけを合流判定に使う。
    left.observed_model == right.observed_model
        && equivalent_model_status(&left_cursor.status, &right_cursor.status)
        && left_cursor.timeleap_runtime == right_cursor.timeleap_runtime
        && left_cursor.stream_cursor == right_cursor.stream_cursor
        && left_cursor.consumption == right_cursor.consumption
}

/// `remain=0`と`remain=1`は、どちらも次のtickでSFMTを1つ読み、
/// `SFMT % 128 == 0`の判定へ進む。同じSFMTカーソルを持つ場合は、
/// 未来の消費・瞬き遷移が同値になるため、合流判定では同じ待機状態とする。
fn equivalent_model_status(left: &ModelStatus, right: &ModelStatus) -> bool {
    left.remain.len() == right.remain.len()
        && left.remain.iter().zip(&right.remain).all(|(left, right)| {
            left == right || (*left == 0 && *right == 1) || (*left == 1 && *right == 0)
        })
}

pub(super) fn parse_nonnegative(text: &str, field: InputField) -> AppResult<i64> {
    let value = text
        .trim()
        .parse::<i64>()
        .map_err(|_| AppError::InvalidInteger(field))?;
    if value < 0 {
        return Err(AppError::NegativeValue(field));
    }
    Ok(value)
}

fn frame1_tick_is_in_research_range(
    frame1_tick: i64,
    nominal_tick: i64,
    range_frames: i64,
) -> bool {
    let tick_radius = range_frames.clamp(0, 98).div_euclid(2) as u64;
    frame1_tick.abs_diff(nominal_tick) <= tick_radius
}

fn valid_frame1_input(value: &str) -> Option<i64> {
    value
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|frame| (0..=crate::domain::rng::MAX_SFMT_FRAME).contains(frame))
}

fn parse_bounded_timeleap_value(
    text: &str,
    minimum: i64,
    maximum: i64,
    label: &str,
) -> Result<i64, String> {
    let value = text
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("{label}を確認してください。"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!(
            "{label}は{minimum}～{maximum}の範囲で指定してください。"
        ));
    }
    Ok(value)
}
