use super::super::rng::{ModelStatus, ObservationCache, Sfmt};
#[cfg(test)]
use super::runtime::advance_no_blink_run_from_values;
#[cfg(test)]
use super::runtime::apply_event;
#[cfg(test)]
use super::runtime::{
    advance_delay_tick, advance_delay_tick_reference, advance_one_tick_reference,
    advance_one_tick_with_active_models, advance_one_tick_with_active_models_traced,
    apply_consider_delay, apply_consider_delay_traced, apply_stationary_delay,
    consider_delay_landing_frame, consider_delay_type_id, consume_courier,
    courier_consumption_from_values, delay_elapsed_tick_bound, normal_target_ticks,
    normal_target_ticks_cancelable,
};
use super::runtime::{
    advance_no_blink_run_from_values_with_index, advance_one_tick,
    advance_one_tick_state_only_from_values, advance_one_tick_without_blink_list,
    advance_to_target, apply_event_with_profile, cached_sfmt_at, validate_config,
    NoBlinkTriggerIndex,
};
use super::ConsiderDelayGenerator;
#[cfg(test)]
use super::CourierConfig;
#[cfg(test)]
use super::TimeleapMode;
#[cfg(test)]
use super::{begin_model_rng_trace, finish_model_rng_trace, ModelRngEventKind};
use super::{
    ConsiderDelayConfig, TimeleapConfig, TimeleapSearchFailure, TimeleapSearchKey,
    TimeleapTimelineStart, MAX_CANDIDATES, MAX_CANDIDATE_REPLAY_TICKS, MAX_NORMAL_TIMELINE_TICKS,
    MAX_PARALLEL_TIMELEAP_WORKERS, MAX_TIMELEAP_MEMO_CACHE_BYTES, MERGE_CHECK_SFMT_INTERVAL,
};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;
// Bounded lookahead amortizes candidate-search setup. Only the contiguous edge
// is retained; after a gap, no subsequent chunk is evaluated.
const GROUP_BOUNDARY_PROBE_CHUNK_TICKS: i64 = 256;
const CANDIDATE_GROUP_CANCEL_INTERVAL: usize = 128;
const CANDIDATE_PREPARATION_BATCH_SIZE: usize = 1_024;
const MAX_SHARED_CONSIDER_DELAY_INPUT_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_CACHED_CANDIDATE_RNG_VALUES: usize = 4 * 1024 * 1024;
const CACHED_CANDIDATE_RNG_TAIL_VALUES: i64 = 260;
mod consider_delay;
mod frame1_origins;
mod frame1_range;
mod group_boundary_completion;
pub(super) use consider_delay::*;
use frame1_origins::{
    resolve_frame1_origin_scope, CandidateSnapshotSource, CandidateTimelineCheckpoint,
    ConsiderDelayCandidateInput, Frame1OriginQuery, Frame1OriginWindow,
};
use frame1_range::Frame1CandidateRange;
use group_boundary_completion::{take_contiguous_prefix, take_contiguous_suffix};

#[cfg(test)]
mod tests;

pub(crate) fn run_timeleap_candidate_search(
    key: &TimeleapSearchKey,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<Vec<TimeleapCandidateGroup>, TimeleapSearchFailure> {
    let required_end = key
        .config
        .target_frame
        .checked_add(311)
        .ok_or_else(|| {
            TimeleapSearchFailure::Calculation("Timeleapのキャッシュ範囲を計算できません。".into())
        })?
        .min(crate::domain::rng::MAX_SFMT_FRAME);
    cache
        .ensure_through(required_end, Some(cancel))
        .map_err(|error| {
            if matches!(error, crate::errors::AppError::Cancelled) {
                TimeleapSearchFailure::Cancelled
            } else {
                TimeleapSearchFailure::Cache(error)
            }
        })?;
    if is_cancelled(Some(cancel)) {
        return Err(TimeleapSearchFailure::Cancelled);
    }
    let groups = search_candidates_from_timeline_with_frame1_filter(
        &key.config,
        &key.timeline_start,
        key.current_tick,
        cache,
        Some(cancel),
        true,
        key.consider_delay.as_ref(),
        true,
        key.frame1_filter,
    )
    .map(|(groups, _)| groups)
    .map_err(|message| {
        if is_cancelled(Some(cancel)) {
            TimeleapSearchFailure::Cancelled
        } else {
            TimeleapSearchFailure::Calculation(message)
        }
    })?;
    Ok(groups
        .into_iter()
        .filter(|group| {
            group.grace_frames >= key.grace_min && group.target_grace_frames >= key.target_grace_min
        })
        .collect())
}

/// Frame1入力値の前後にある通常Timeline上の到達可能Frameから、
/// イベント適用後の実行候補を作る。Target到達可否は候補条件にしない。
pub(crate) fn run_timeleap_frame1_preparation(
    config: &TimeleapConfig,
    nominal_frame1: i64,
    start: &TimeleapTimelineStart,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<TimeleapFrame1Preparation, TimeleapSearchFailure> {
    validate_config(config).map_err(TimeleapSearchFailure::Calculation)?;
    if cache.seed != config.seed {
        return Err(TimeleapSearchFailure::Calculation(
            "Timeleap用のSFMTキャッシュが現在のSeedと一致しません。".into(),
        ));
    }
    // The UI range is expressed in display Frames, while candidates are Timeline ticks.
    // Integer division floors odd values because a half-tick candidate cannot exist.
    let tick_radius = config.handoff_range_frames.max(0) / 2;
    let max_consumption_per_tick = config.models as i64 + 3;
    let ticks_to_frame1 = nominal_frame1.saturating_sub(start.frame).max(0);
    let required_end = start
        .sfmt_position
        .saturating_add(ticks_to_frame1)
        .saturating_add(tick_radius.saturating_mul(max_consumption_per_tick))
        .checked_add(311)
        .ok_or_else(|| {
            TimeleapSearchFailure::Calculation("Frame1のキャッシュ範囲を計算できません。".into())
        })?
        .min(crate::domain::rng::MAX_SFMT_FRAME);
    cache
        .ensure_through(required_end, Some(cancel))
        .map_err(|error| {
            if matches!(error, crate::errors::AppError::Cancelled) {
                TimeleapSearchFailure::Cancelled
            } else {
                TimeleapSearchFailure::Cache(error)
            }
        })?;
    if is_cancelled(Some(cancel)) {
        return Err(TimeleapSearchFailure::Cancelled);
    }
    if nominal_frame1 < config.start_frame || nominal_frame1 > crate::domain::rng::MAX_SFMT_FRAME {
        return Err(TimeleapSearchFailure::Frame1Unreachable);
    }
    if start.sfmt_position < 0 || start.model.remain.len() != config.models {
        return Err(TimeleapSearchFailure::Calculation(
            "Frame1の通常Timeline開始状態が一致しません。".into(),
        ));
    }
    if start.frame > nominal_frame1 {
        return Err(TimeleapSearchFailure::Frame1Unreachable);
    }
    let mut sfmt =
        cached_sfmt_at(cache, start.sfmt_position).map_err(TimeleapSearchFailure::Calculation)?;
    let mut status = TimeleapStatus::new(config.models);
    status.model = start.model.clone();
    status.is_boy = config.is_boy;
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut preceding_ticks = VecDeque::with_capacity(tick_radius as usize + 1);
    let mut nominal_tick = None;
    loop {
        if is_cancelled(Some(cancel)) {
            return Err(TimeleapSearchFailure::Cancelled);
        }
        if cursor.tick >= MAX_NORMAL_TIMELINE_TICKS as i64 {
            return Err(TimeleapSearchFailure::Calculation(
                "Frame1までの通常Timelineが検索上限を超えました。".into(),
            ));
        }
        if cursor.frame == nominal_frame1 {
            nominal_tick = Some(cursor.tick);
            preceding_ticks.push_back(Frame1TimelineSnapshot {
                cursor,
                status: status.clone(),
                sfmt: sfmt.clone(),
            });
            while preceding_ticks.len() > tick_radius as usize + 1 {
                preceding_ticks.pop_front();
            }
            break;
        }
        if cursor.frame > nominal_frame1 {
            break;
        }
        preceding_ticks.push_back(Frame1TimelineSnapshot {
            cursor,
            status: status.clone(),
            sfmt: sfmt.clone(),
        });
        if preceding_ticks.len() > tick_radius as usize + 1 {
            preceding_ticks.pop_front();
        }
        advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
    }
    let Some(nominal_tick) = nominal_tick else {
        return Err(TimeleapSearchFailure::Frame1Unreachable);
    };
    let mut candidates = Vec::with_capacity(tick_radius as usize * 2 + 1);
    for snapshot in preceding_ticks {
        if is_cancelled(Some(cancel)) {
            return Err(TimeleapSearchFailure::Cancelled);
        }
        push_frame1_event_candidate(config, snapshot, &mut candidates);
    }
    let last_tick = nominal_tick
        .saturating_add(tick_radius)
        .min(MAX_NORMAL_TIMELINE_TICKS as i64);
    while cursor.tick < last_tick {
        if is_cancelled(Some(cancel)) {
            return Err(TimeleapSearchFailure::Cancelled);
        }
        advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
        push_frame1_event_candidate(
            config,
            Frame1TimelineSnapshot {
                cursor,
                status: status.clone(),
                sfmt: sfmt.clone(),
            },
            &mut candidates,
        );
    }
    Ok(TimeleapFrame1Preparation {
        nominal_frame1,
        nominal_tick,
        candidates,
    })
}

/// Check whether the entered TargetFrame is reached after applying the selected
/// Timeleap event at the exact Frame1. This is the normal field Target check and
/// deliberately does not run Consider Delay.
pub(crate) fn timeleap_target_reachable_after_frame1(
    preparation: &TimeleapFrame1Preparation,
    frame1: i64,
    target_frame: i64,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<bool, TimeleapSearchFailure> {
    if !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&target_frame) {
        return Err(TimeleapSearchFailure::Calculation(
            "TargetFrameの入力範囲を確認してください。".into(),
        ));
    }
    let Some(candidate) = preparation
        .candidates
        .iter()
        .find(|candidate| candidate.frame1 == frame1)
    else {
        return Ok(false);
    };
    if candidate.post_frame > target_frame {
        return Ok(false);
    }

    // Bound cache growth by the maximum SFMT consumption per tick between the
    // post-event cursor and the requested target. The UI worker can extend the
    // shared cache without blocking the Frame1 timer.
    let required_end = candidate
        .post_sfmt_position
        // `cursor.frame` and `cursor.sfmt_position` advance by the same number
        // of consumed SFMT values. Sleeping ticks may add time without cache
        // reads, so multiplying by model count would over-extend needlessly.
        .saturating_add(target_frame.saturating_sub(candidate.post_frame))
        .saturating_add(311)
        .min(crate::domain::rng::MAX_SFMT_FRAME);
    cache
        .ensure_through(required_end, Some(cancel))
        .map_err(|error| {
            if matches!(error, crate::errors::AppError::Cancelled) {
                TimeleapSearchFailure::Cancelled
            } else {
                TimeleapSearchFailure::Cache(error)
            }
        })?;
    if is_cancelled(Some(cancel)) {
        return Err(TimeleapSearchFailure::Cancelled);
    }

    let mut cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: candidate.post_tick,
    };
    let mut status = candidate.post_status.clone();
    let mut sfmt =
        cached_sfmt_at(cache, cursor.sfmt_position).map_err(TimeleapSearchFailure::Calculation)?;
    let mut suffixes = HashMap::new();
    advance_to_target_with_merging(
        target_frame,
        &mut status,
        &mut cursor,
        &mut sfmt,
        &mut suffixes,
        Some(cancel),
        &mut TimeleapSearchStats::default(),
    )
    .map_err(|message| {
        if is_cancelled(Some(cancel)) {
            TimeleapSearchFailure::Cancelled
        } else {
            TimeleapSearchFailure::Calculation(message)
        }
    })
}

pub(super) struct Frame1TimelineSnapshot {
    cursor: Cursor,
    status: TimeleapStatus,
    sfmt: Sfmt,
}

pub(super) fn push_frame1_event_candidate(
    config: &TimeleapConfig,
    snapshot: Frame1TimelineSnapshot,
    candidates: &mut Vec<TimeleapCandidate>,
) {
    let mut candidate_status = snapshot.status;
    let mut candidate_cursor = snapshot.cursor;
    let mut candidate_sfmt = snapshot.sfmt;
    let pre_frame = candidate_cursor.frame;
    let pre_sfmt_position = candidate_cursor.sfmt_position;
    let pre_status = candidate_status.clone();
    apply_event_with_profile(
        &mut candidate_status,
        &mut candidate_cursor,
        &mut candidate_sfmt,
        config.mode,
        &config.courier.generation_profile(),
    );
    candidates.push(TimeleapCandidate {
        frame1: snapshot.cursor.frame,
        frame1_tick: snapshot.cursor.tick,
        post_tick: candidate_cursor.tick,
        grace_frames: 0,
        pre_frame,
        pre_sfmt_position,
        pre_status,
        post_frame: candidate_cursor.frame,
        post_sfmt_position: candidate_cursor.sfmt_position,
        post_status: candidate_status,
        target_inputs: vec![TimeleapTargetInput {
            frame: candidate_cursor.frame,
            tick: candidate_cursor.tick,
        }]
        .into(),
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapFrame1Preparation {
    pub nominal_frame1: i64,
    pub nominal_tick: i64,
    pub candidates: Vec<TimeleapCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapCandidate {
    pub frame1: i64,
    pub frame1_tick: i64,
    /// Frame2へ到達したTimeline tick。Frame1以降に操作できる実時間の算出に使う。
    pub post_tick: i64,
    pub grace_frames: i64,
    /// bak1: イベント適用直前のFrame1状態。
    pub pre_frame: i64,
    pub pre_sfmt_position: i64,
    pub pre_status: TimeleapStatus,
    /// bak2: イベント後にTargetへ到達した状態。
    pub post_frame: i64,
    pub post_sfmt_position: i64,
    /// Immutable state shared by Frame1 origins restored from the same merged timeline.
    pub post_status: TimeleapStatus,
    /// Consider Delay後に生成開始Frameへ着地できるA入力位置。
    pub target_inputs: Arc<[TimeleapTargetInput]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapTargetInput {
    pub frame: i64,
    pub tick: i64,
}

impl TimeleapCandidate {
    pub(crate) fn frame1_to_frame2_ticks(&self) -> i64 {
        self.target_inputs
            .get(self.target_inputs.len() / 2)
            .map(|input| input.tick.saturating_sub(self.frame1_tick).max(0))
            .unwrap_or_else(|| self.post_tick.saturating_sub(self.frame1_tick).max(0))
    }
}

/// TimelineLeapだけが持つイベント状態。通常の孵化後観測のModelStatusへ
/// fidgetや天候の意味を混ぜず、参照ツールのイベント処理をこの境界に閉じ込める。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeleapStatus {
    pub model: ModelStatus,
    pub phase: i32,
    pub fidget_cd: i32,
    pub is_boy: bool,
    pub raining: bool,
}

/// Timeleap固有の状態を通常Timelineへ継続するための状態。
/// モデルのremainは`TimelineCursor::status`側で保持する。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeleapContinuation {
    pub phase: i32,
    pub fidget_cd: i32,
    pub is_boy: bool,
    pub raining: bool,
}

impl From<&TimeleapStatus> for TimeleapContinuation {
    fn from(status: &TimeleapStatus) -> Self {
        Self {
            phase: status.phase,
            fidget_cd: status.fidget_cd,
            is_boy: status.is_boy,
            raining: status.raining,
        }
    }
}

impl TimeleapStatus {
    pub(in crate::domain::timeleap) fn new(models: usize) -> Self {
        Self {
            model: ModelStatus::new(models),
            phase: 0,
            fidget_cd: -1,
            is_boy: false,
            raining: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapCandidateGroup {
    pub frame1_start: i64,
    pub frame1_end: i64,
    pub frame1_tick_start: i64,
    pub frame1_tick_end: i64,
    pub representative_frame1: i64,
    /// 表示用に選ばれた代表候補のFrame1→Frame2経過tick。
    pub frame2_ticks: i64,
    pub grace_frames: i64,
    pub target_frame_start: i64,
    pub target_frame_end: i64,
    pub target_tick_start: i64,
    pub target_tick_end: i64,
    pub representative_target_frame: i64,
    pub target_grace_frames: i64,
    pub candidates: smallvec::SmallVec<[TimeleapCandidate; 1]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimeleapSelection {
    pub representative_frame1: i64,
    pub frame1_tick: i64,
    pub post_sfmt_position: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct TimeleapShiftMatch {
    pub candidate_index: usize,
    pub distance: i64,
}

/// 瞬き補正後のShift時刻列を、参照側が保持した絶対時刻列へ照合する。
/// 候補を無限に待たず、呼び出し側が渡した有限列だけを対象にする。
#[cfg(test)]
pub(crate) fn match_shift_times(
    predicted: &[Vec<i64>],
    observed: &[i64],
    correction: i64,
    tolerance: i64,
) -> Vec<TimeleapShiftMatch> {
    if observed.is_empty() || tolerance < 0 {
        return Vec::new();
    }
    predicted
        .iter()
        .enumerate()
        .filter_map(|(candidate_index, times)| {
            if times.len() != observed.len() {
                return None;
            }
            let distance = observed
                .iter()
                .zip(times.iter())
                .map(|(actual, expected)| (actual.saturating_sub(correction) - expected).abs())
                .max()?;
            (distance <= tolerance).then_some(TimeleapShiftMatch {
                candidate_index,
                distance,
            })
        })
        .collect()
}

/// SetされたFrame1が現在のスナップショットに残っていればそれを優先し、
/// 残っていなければ候補群の中央（偶数幅では後ろ側）を採用する。
pub(crate) fn select_candidate(
    groups: &[TimeleapCandidateGroup],
    selected_frame1: Option<i64>,
) -> Option<TimeleapSelection> {
    let mut candidates = groups
        .iter()
        .flat_map(|group| group.candidates.iter())
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|candidate| candidate.frame1);
    candidates.dedup_by_key(|candidate| candidate.frame1);

    let selected_is_in_a_frame1_run =
        selected_frame1.is_some_and(|frame| groups.iter().any(|group| group.contains_frame(frame)));
    let candidate = if let Some(frame) = selected_frame1 {
        candidates
            .iter()
            .copied()
            .find(|candidate| candidate.frame1 == frame)
            .or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .min_by_key(|candidate| ((candidate.frame1 - frame).abs(), -candidate.frame1))
            })?
    } else {
        *candidates.get(candidates.len() / 2)?
    };
    let representative = if selected_is_in_a_frame1_run {
        selected_frame1.unwrap_or(candidate.frame1)
    } else {
        candidate.frame1
    };
    Some(TimeleapSelection {
        representative_frame1: representative,
        frame1_tick: candidate.frame1_tick,
        post_sfmt_position: candidate.post_sfmt_position,
    })
}

#[cfg(test)]
pub(crate) fn candidates_have_same_future_state(
    left: &TimeleapCandidate,
    right: &TimeleapCandidate,
) -> bool {
    left.post_frame == right.post_frame
        && left.post_sfmt_position == right.post_sfmt_position
        && left.post_tick == right.post_tick
        && left.post_status == right.post_status
}

/// 観測列に一致する予測窓ごとに、最後の瞬き直後の継続状態を返す。
/// 許容差によって同一候補内の複数位置が一致した場合も、異なる状態を潰さない。
#[cfg(test)]
pub(crate) fn matching_shift_continuations(
    cache: &ObservationCache,
    candidate: &TimeleapCandidate,
    model: usize,
    observed: &[i64],
    correction: i64,
    tolerance: i64,
    until_frame: i64,
) -> Vec<TimeleapCandidate> {
    matching_shift_continuations_cancelable(
        cache,
        candidate,
        model,
        observed,
        correction,
        tolerance,
        until_frame,
        None,
    )
    .unwrap_or_default()
}

/// Cancellable visitor used by the UI worker. `None` indicates cancellation;
/// `Some(false)` means the caller has enough matches and asked to stop early.
#[cfg(test)]
pub(crate) fn matching_shift_continuations_cancelable(
    cache: &ObservationCache,
    candidate: &TimeleapCandidate,
    model: usize,
    observed: &[i64],
    correction: i64,
    tolerance: i64,
    until_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Option<Vec<TimeleapCandidate>> {
    let mut matches = Vec::new();
    let _ = visit_matching_shift_continuations_cancelable(
        cache,
        candidate,
        model,
        observed,
        correction,
        tolerance,
        until_frame,
        cancel,
        |continuation| {
            matches.push(continuation);
            true
        },
    )?;
    Some(matches)
}

pub(crate) fn visit_matching_shift_continuations_cancelable(
    cache: &ObservationCache,
    candidate: &TimeleapCandidate,
    model: usize,
    observed: &[i64],
    correction: i64,
    tolerance: i64,
    until_frame: i64,
    cancel: Option<&AtomicBool>,
    mut visit: impl FnMut(TimeleapCandidate) -> bool,
) -> Option<bool> {
    if observed.is_empty() || until_frame < candidate.post_tick.saturating_mul(2) {
        return Some(true);
    }

    let mut status = candidate.post_status.clone();
    let mut cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: candidate.post_tick,
    };
    let Ok(mut sfmt) = cached_sfmt_at(cache, candidate.post_sfmt_position) else {
        return Some(true);
    };
    let mut recent_times = VecDeque::with_capacity(observed.len());
    for _ in 0..MAX_NORMAL_TIMELINE_TICKS {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return None;
        }
        if cursor.tick.saturating_mul(2) > until_frame {
            break;
        }
        let blinks = advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        if blinks.contains(&(model as u8)) {
            recent_times.push_back(cursor.tick.saturating_mul(2));
            if recent_times.len() > observed.len() {
                recent_times.pop_front();
            }
            if recent_times.len() == observed.len() {
                if shift_time_window_matches(&recent_times, observed, correction, tolerance) {
                    let mut continuation = candidate.clone();
                    continuation.post_frame = cursor.frame;
                    continuation.post_sfmt_position = cursor.sfmt_position;
                    continuation.post_tick = cursor.tick;
                    continuation.post_status = status.clone();
                    if !visit(continuation) {
                        return Some(false);
                    }
                }
            }
        }
    }
    Some(true)
}

pub(super) fn shift_time_window_matches(
    predicted: &VecDeque<i64>,
    observed: &[i64],
    correction: i64,
    tolerance: i64,
) -> bool {
    predicted.len() == observed.len()
        && tolerance >= 0
        && observed.iter().zip(predicted).all(|(actual, expected)| {
            actual
                .saturating_sub(correction)
                .saturating_sub(*expected)
                .saturating_abs()
                <= tolerance
        })
}

#[cfg(test)]
pub(crate) fn select_surviving_frame1(
    candidates: &[&TimeleapCandidate],
    selected_frame1: Option<i64>,
) -> Option<i64> {
    if candidates.is_empty() {
        return None;
    }
    if let Some(selected) = selected_frame1 {
        if candidates
            .iter()
            .any(|candidate| candidate.frame1 == selected)
        {
            return Some(selected);
        }
    }
    let mut ordered: Vec<i64> = candidates
        .iter()
        .map(|candidate| candidate.frame1)
        .collect();
    ordered.sort_unstable();
    Some(ordered[ordered.len() / 2])
}

impl TimeleapCandidateGroup {
    fn contains_frame(&self, frame: i64) -> bool {
        (self.frame1_start..=self.frame1_end).contains(&frame)
    }

    pub(crate) fn representative_candidate(&self) -> Option<&TimeleapCandidate> {
        self.candidates.get(self.candidates.len() / 2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cursor {
    pub(super) frame: i64,
    pub(super) sfmt_position: i64,
    pub(super) tick: i64,
}

#[derive(Clone)]
pub(super) struct Frame1CandidateOrigin {
    frame1: i64,
    frame1_tick: i64,
    pre_frame: i64,
    pre_sfmt_position: i64,
    pre_status: TimeleapStatus,
    scan_start_tick: i64,
}

pub(super) struct MergedConsiderDelayTimeline {
    representative_scan_tick: i64,
    scan_status: TimeleapStatus,
    scan_cursor: Cursor,
    origins: Vec<Frame1CandidateOrigin>,
}

pub(super) fn restore_shared_timeline_result(
    origins: Vec<Frame1CandidateOrigin>,
    representative_scan_tick: i64,
    hits: Vec<ConsiderDelayHit>,
) -> Vec<TimeleapCandidate> {
    let representative = hits.get(hits.len() / 2);
    let Some(representative) = representative else {
        return Vec::new();
    };
    origins
        .into_iter()
        .map(|origin| TimeleapCandidate {
            frame1: origin.frame1,
            frame1_tick: origin.frame1_tick,
            post_tick: origin.scan_start_tick.saturating_add(
                representative
                    .landing_cursor
                    .tick
                    .saturating_sub(representative_scan_tick),
            ),
            grace_frames: 0,
            pre_frame: origin.pre_frame,
            pre_sfmt_position: origin.pre_sfmt_position,
            pre_status: origin.pre_status,
            post_frame: representative.landing_cursor.frame,
            post_sfmt_position: representative.landing_cursor.sfmt_position,
            post_status: representative.landing_status.clone(),
            target_inputs: hits
                .iter()
                .map(|hit| TimeleapTargetInput {
                    frame: hit.input_cursor.frame,
                    tick: origin.scan_start_tick.saturating_add(
                        hit.input_cursor
                            .tick
                            .saturating_sub(representative_scan_tick),
                    ),
                })
                .collect::<Vec<_>>()
                .into(),
        })
        .collect()
}

#[derive(Clone)]
pub(super) struct ConsiderDelayHit {
    pub(super) input_cursor: Cursor,
    pub(super) landing_cursor: Cursor,
    pub(super) landing_status: TimeleapStatus,
}

/// 参照Timelineの通常状態をTargetまで進め、厳密着地するか返す。
#[cfg(test)]
pub(crate) fn search_candidates_from_cache(
    config: &TimeleapConfig,
    cache: &ObservationCache,
) -> Result<Vec<TimeleapCandidateGroup>, String> {
    search_candidates_from_cache_cancelable(config, cache, None)
}

#[cfg(test)]
pub(crate) fn search_candidates_from_cache_cancelable(
    config: &TimeleapConfig,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<TimeleapCandidateGroup>, String> {
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);
    search_candidates_from_timeline_cancelable(config, &start, 0, cache, cancel)
}

#[cfg(test)]
pub(super) fn search_candidates_from_timeline_cancelable(
    config: &TimeleapConfig,
    timeline_start: &TimeleapTimelineStart,
    current_tick: i64,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<TimeleapCandidateGroup>, String> {
    search_candidates_from_timeline_with_stats(
        config,
        timeline_start,
        current_tick,
        cache,
        cancel,
        true,
        None,
        true,
    )
    .map(|(groups, _)| groups)
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct TimeleapSearchStats {
    normal_ticks_scanned: u64,
    scope_resolution_ticks: u64,
    candidates_tested: u64,
    core_frame1_origin_ticks: u64,
    boundary_frame1_origin_ticks: u64,
    boundary_probe_ticks: u64,
    post_event_snapshots: u64,
    merged_timeline_work_items: u64,
    candidate_ticks_simulated: u64,
    frame1_to_a_ticks_simulated: u64,
    a_window_ticks_simulated: u64,
    frame1_enumeration_micros: u64,
    a_input_search_micros: u64,
    candidate_restore_micros: u64,
    #[cfg(test)]
    candidate_snapshot_micros: u64,
    #[cfg(test)]
    candidate_preparation_wall_micros: u64,
    #[cfg(test)]
    candidate_grouping_micros: u64,
    merge_hits: u64,
    shared_suffix_cache_hits: u64,
    shared_suffix_cache_misses: u64,
    shared_suffix_cache_bypasses: u64,
    shared_suffix_cache_waits: u64,
    shared_suffix_cache_states: usize,
    shared_cache_evictions: u64,
    shared_cache_peak_bytes: usize,
    consider_delay_scan_reuse_hits: u64,
    consider_delay_scan_misses: u64,
    consider_delay_scan_bypasses: u64,
    consider_delay_scan_waits: u64,
    consider_delay_scan_cached_states: usize,
    consider_delay_input_window_cache_hits: u64,
    consider_delay_input_window_cache_misses: u64,
    consider_delay_input_suffix_cache_hits: u64,
    shared_consider_delay_input_result_hits: u64,
    shared_consider_delay_input_result_misses: u64,
    shared_consider_delay_input_result_waits: u64,
    shared_consider_delay_input_result_bypasses: u64,
    shared_consider_delay_input_suffix_hits: u64,
    shared_consider_delay_input_suffix_misses: u64,
    shared_consider_delay_input_suffix_waits: u64,
    shared_consider_delay_input_suffix_bypasses: u64,
    shared_consider_delay_input_suffix_states: usize,
    shared_consider_delay_input_suffix_peak_bytes: usize,
    event_generation_bound_micros: u64,
    event_generation_max_outputs: u64,
    remembered_checkpoints: usize,
}

impl TimeleapSearchStats {
    fn add_candidate_stats(&mut self, other: Self) {
        self.normal_ticks_scanned = self
            .normal_ticks_scanned
            .saturating_add(other.normal_ticks_scanned);
        self.scope_resolution_ticks = self
            .scope_resolution_ticks
            .saturating_add(other.scope_resolution_ticks);
        self.candidates_tested = self
            .candidates_tested
            .saturating_add(other.candidates_tested);
        self.core_frame1_origin_ticks = self
            .core_frame1_origin_ticks
            .saturating_add(other.core_frame1_origin_ticks);
        self.boundary_frame1_origin_ticks = self
            .boundary_frame1_origin_ticks
            .saturating_add(other.boundary_frame1_origin_ticks);
        self.boundary_probe_ticks = self
            .boundary_probe_ticks
            .saturating_add(other.boundary_probe_ticks);
        self.post_event_snapshots = self
            .post_event_snapshots
            .saturating_add(other.post_event_snapshots);
        self.merged_timeline_work_items = self
            .merged_timeline_work_items
            .saturating_add(other.merged_timeline_work_items);
        self.candidate_ticks_simulated = self
            .candidate_ticks_simulated
            .saturating_add(other.candidate_ticks_simulated);
        self.frame1_to_a_ticks_simulated = self
            .frame1_to_a_ticks_simulated
            .saturating_add(other.frame1_to_a_ticks_simulated);
        self.a_window_ticks_simulated = self
            .a_window_ticks_simulated
            .saturating_add(other.a_window_ticks_simulated);
        self.frame1_enumeration_micros = self
            .frame1_enumeration_micros
            .saturating_add(other.frame1_enumeration_micros);
        self.a_input_search_micros = self
            .a_input_search_micros
            .saturating_add(other.a_input_search_micros);
        self.candidate_restore_micros = self
            .candidate_restore_micros
            .saturating_add(other.candidate_restore_micros);
        #[cfg(test)]
        {
            self.candidate_snapshot_micros = self
                .candidate_snapshot_micros
                .saturating_add(other.candidate_snapshot_micros);
            self.candidate_preparation_wall_micros = self
                .candidate_preparation_wall_micros
                .saturating_add(other.candidate_preparation_wall_micros);
            self.candidate_grouping_micros = self
                .candidate_grouping_micros
                .saturating_add(other.candidate_grouping_micros);
        }
        self.merge_hits = self.merge_hits.saturating_add(other.merge_hits);
        self.shared_suffix_cache_hits = self
            .shared_suffix_cache_hits
            .saturating_add(other.shared_suffix_cache_hits);
        self.shared_suffix_cache_misses = self
            .shared_suffix_cache_misses
            .saturating_add(other.shared_suffix_cache_misses);
        self.shared_suffix_cache_bypasses = self
            .shared_suffix_cache_bypasses
            .saturating_add(other.shared_suffix_cache_bypasses);
        self.shared_suffix_cache_waits = self
            .shared_suffix_cache_waits
            .saturating_add(other.shared_suffix_cache_waits);
        self.shared_suffix_cache_states = self
            .shared_suffix_cache_states
            .saturating_add(other.shared_suffix_cache_states);
        self.shared_cache_peak_bytes = self
            .shared_cache_peak_bytes
            .max(other.shared_cache_peak_bytes);
        self.consider_delay_scan_reuse_hits = self
            .consider_delay_scan_reuse_hits
            .saturating_add(other.consider_delay_scan_reuse_hits);
        self.consider_delay_scan_misses = self
            .consider_delay_scan_misses
            .saturating_add(other.consider_delay_scan_misses);
        self.consider_delay_scan_bypasses = self
            .consider_delay_scan_bypasses
            .saturating_add(other.consider_delay_scan_bypasses);
        self.consider_delay_scan_waits = self
            .consider_delay_scan_waits
            .saturating_add(other.consider_delay_scan_waits);
        self.consider_delay_scan_cached_states = self
            .consider_delay_scan_cached_states
            .saturating_add(other.consider_delay_scan_cached_states);
        self.consider_delay_input_window_cache_hits = self
            .consider_delay_input_window_cache_hits
            .saturating_add(other.consider_delay_input_window_cache_hits);
        self.consider_delay_input_window_cache_misses = self
            .consider_delay_input_window_cache_misses
            .saturating_add(other.consider_delay_input_window_cache_misses);
        self.consider_delay_input_suffix_cache_hits = self
            .consider_delay_input_suffix_cache_hits
            .saturating_add(other.consider_delay_input_suffix_cache_hits);
        self.shared_consider_delay_input_result_hits = self
            .shared_consider_delay_input_result_hits
            .saturating_add(other.shared_consider_delay_input_result_hits);
        self.shared_consider_delay_input_result_misses = self
            .shared_consider_delay_input_result_misses
            .saturating_add(other.shared_consider_delay_input_result_misses);
        self.shared_consider_delay_input_result_waits = self
            .shared_consider_delay_input_result_waits
            .saturating_add(other.shared_consider_delay_input_result_waits);
        self.shared_consider_delay_input_result_bypasses = self
            .shared_consider_delay_input_result_bypasses
            .saturating_add(other.shared_consider_delay_input_result_bypasses);
        self.shared_consider_delay_input_suffix_hits = self
            .shared_consider_delay_input_suffix_hits
            .saturating_add(other.shared_consider_delay_input_suffix_hits);
        self.shared_consider_delay_input_suffix_misses = self
            .shared_consider_delay_input_suffix_misses
            .saturating_add(other.shared_consider_delay_input_suffix_misses);
        self.shared_consider_delay_input_suffix_waits = self
            .shared_consider_delay_input_suffix_waits
            .saturating_add(other.shared_consider_delay_input_suffix_waits);
        self.shared_consider_delay_input_suffix_bypasses = self
            .shared_consider_delay_input_suffix_bypasses
            .saturating_add(other.shared_consider_delay_input_suffix_bypasses);
        self.shared_consider_delay_input_suffix_states = self
            .shared_consider_delay_input_suffix_states
            .saturating_add(other.shared_consider_delay_input_suffix_states);
        self.shared_consider_delay_input_suffix_peak_bytes = self
            .shared_consider_delay_input_suffix_peak_bytes
            .max(other.shared_consider_delay_input_suffix_peak_bytes);
        self.event_generation_bound_micros = self
            .event_generation_bound_micros
            .saturating_add(other.event_generation_bound_micros);
        self.event_generation_max_outputs = self
            .event_generation_max_outputs
            .max(other.event_generation_max_outputs);
        self.remembered_checkpoints = self
            .remembered_checkpoints
            .saturating_add(other.remembered_checkpoints);
    }
}

#[cfg(test)]
pub(super) fn search_candidates_from_cache_with_stats(
    config: &TimeleapConfig,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
    merge_future_states: bool,
) -> Result<(Vec<TimeleapCandidateGroup>, TimeleapSearchStats), String> {
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);
    search_candidates_from_timeline_with_stats(
        config,
        &start,
        0,
        cache,
        cancel,
        merge_future_states,
        None,
        false,
    )
}

#[cfg(test)]
pub(super) fn search_candidates_from_timeline_with_stats(
    config: &TimeleapConfig,
    timeline_start: &TimeleapTimelineStart,
    current_tick: i64,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
    merge_future_states: bool,
    consider_delay: Option<&ConsiderDelayConfig>,
    parallelize_consider_delay: bool,
) -> Result<(Vec<TimeleapCandidateGroup>, TimeleapSearchStats), String> {
    search_candidates_from_timeline_with_frame1_filter(
        config,
        timeline_start,
        current_tick,
        cache,
        cancel,
        merge_future_states,
        consider_delay,
        parallelize_consider_delay,
        None,
    )
}

fn search_candidates_from_timeline_with_frame1_filter(
    config: &TimeleapConfig,
    timeline_start: &TimeleapTimelineStart,
    current_tick: i64,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
    merge_future_states: bool,
    consider_delay: Option<&ConsiderDelayConfig>,
    parallelize_consider_delay: bool,
    frame1_filter: Option<(i64, i64)>,
) -> Result<(Vec<TimeleapCandidateGroup>, TimeleapSearchStats), String> {
    validate_config(config)?;
    if cache.seed != config.seed {
        return Err("Timeleap用のSFMTキャッシュが現在のSeedと一致しません。".into());
    }
    if timeline_start.model.remain.len() != config.models
        || timeline_start.tick > current_tick
        || timeline_start.frame > config.start_frame
        || timeline_start.sfmt_position < 0
    {
        return Err("通常Timelineの検索開始状態が一致しません。".into());
    }
    let mut sfmt = cached_sfmt_at(cache, timeline_start.sfmt_position)?;
    let mut status = TimeleapStatus::new(config.models);
    status.model = timeline_start.model.clone();
    status.is_boy = config.is_boy;
    let mut cursor = Cursor {
        frame: timeline_start.frame,
        sfmt_position: timeline_start.sfmt_position,
        tick: timeline_start.tick,
    };
    while cursor.tick < current_tick {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        if cursor.tick.saturating_sub(timeline_start.tick) >= MAX_NORMAL_TIMELINE_TICKS as i64 {
            return Err("通常Timelineの検索開始状態が検索上限を超えました。".into());
        }
        advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
    }
    // Candidate ticks are relative to the current-position snapshot, but the
    // expensive origin evaluation starts only at the resolved input boundary.
    cursor.tick = 0;
    let base_cursor = cursor;
    let mut stats = TimeleapSearchStats::default();
    if config.target_frame <= cursor.frame {
        return Ok((Vec::new(), stats));
    }
    let origin_window = match frame1_filter {
        Some((min_frame, max_frame)) => Frame1OriginWindow::AbsoluteFrames {
            min_frame,
            max_frame,
        },
        None => Frame1OriginWindow::ElapsedTicks {
            min_tick: config.min_frame1_elapsed_ticks,
            max_tick: config.max_frame1_elapsed_ticks,
        },
    };
    let scope = resolve_frame1_origin_scope(
        Frame1OriginQuery {
            target_frame: config.target_frame,
            window: origin_window,
        },
        cursor,
        status,
        sfmt,
        cancel,
    )?;
    let range = scope.range;
    stats.scope_resolution_ticks = scope.scope_resolution_ticks;
    if range.is_empty() {
        return Ok((Vec::new(), stats));
    }
    let start_tick = range.start_tick;
    let end_tick = range.end_tick;
    let event_generation_started = std::time::Instant::now();
    let event_generation_bounds = if let Some(delay) = consider_delay {
        max_event_generation_consumption(cache, delay, config, cursor, cancel)?
    } else {
        EventGenerationFrameBounds::default()
    };
    stats.event_generation_bound_micros = event_generation_started.elapsed().as_micros() as u64;
    stats.event_generation_max_outputs = event_generation_bounds.max.max(0) as u64;

    if parallelize_consider_delay {
        if let Some(delay) = consider_delay {
            if !merge_future_states {
                return Err("Consider Delay検索では候補Timelineの合流計算が必要です。".into());
            }
            let shared_caches = ConsiderDelaySearchCaches::new(
                config.models,
                cache,
                base_cursor,
                config.target_frame,
            )?;
            let core_start_sfmt = cached_sfmt_at(cache, scope.core_start.cursor.sfmt_position)?;
            let (mut candidates, delay_stats, target_reached) =
                search_consider_delay_candidates_parallel_with_caches(
                    config,
                    delay,
                    start_tick,
                    end_tick,
                    scope.core_start.status.clone(),
                    scope.core_start.cursor,
                    core_start_sfmt,
                    cache,
                    event_generation_bounds,
                    &shared_caches,
                    cancel,
                )?;
            stats.core_frame1_origin_ticks = delay_stats.normal_ticks_scanned;
            stats.add_candidate_stats(delay_stats);
            let core_touches_lower = candidates
                .iter()
                .any(|candidate| candidate.frame1_tick == start_tick);
            let core_touches_upper = candidates
                .iter()
                .any(|candidate| candidate.frame1_tick.saturating_add(1) == end_tick);

            if core_touches_lower && start_tick > 0 {
                let mut boundary_end = start_tick;
                let checkpoints = scope
                    .lower_checkpoints
                    .iter()
                    .rev()
                    .filter(|checkpoint| checkpoint.cursor.tick < start_tick)
                    .cloned()
                    .collect::<Vec<_>>();
                for checkpoint in checkpoints {
                    if is_cancelled(cancel) {
                        return Err("Timeleap候補検索がキャンセルされました。".into());
                    }
                    if checkpoint.cursor.tick >= boundary_end {
                        continue;
                    }
                    let chunk = Frame1CandidateRange::new(checkpoint.cursor.tick, boundary_end);
                    let checkpoint_sfmt = cached_sfmt_at(cache, checkpoint.cursor.sfmt_position)?;
                    let (chunk_candidates, chunk_stats, _) =
                        search_consider_delay_candidates_parallel_with_caches(
                            config,
                            delay,
                            chunk.start_tick,
                            chunk.end_tick,
                            checkpoint.status,
                            checkpoint.cursor,
                            checkpoint_sfmt,
                            cache,
                            event_generation_bounds,
                            &shared_caches,
                            cancel,
                        )?;
                    stats.boundary_probe_ticks = stats
                        .boundary_probe_ticks
                        .saturating_add(chunk_stats.normal_ticks_scanned);
                    stats.add_candidate_stats(chunk_stats);
                    let boundary = take_contiguous_suffix(chunk_candidates, chunk);
                    stats.boundary_frame1_origin_ticks = stats
                        .boundary_frame1_origin_ticks
                        .saturating_add(boundary.connected_ticks);
                    let reaches_prior_checkpoint = boundary.reaches_chunk_edge;
                    candidates.extend(boundary.candidates);
                    if candidates.len() >= MAX_CANDIDATES {
                        return Err("Timeleap候補が多すぎます。検索範囲を狭めてください。".into());
                    }
                    if !reaches_prior_checkpoint {
                        break;
                    }
                    boundary_end = chunk.start_tick;
                }
            }

            if core_touches_upper && !target_reached && end_tick < MAX_NORMAL_TIMELINE_TICKS as i64
            {
                let mut extension_status = scope.core_end.status.clone();
                let mut extension_cursor = scope.core_end.cursor;
                let mut extension_sfmt = cached_sfmt_at(cache, extension_cursor.sfmt_position)?;
                let mut extension_start_tick = end_tick;
                while extension_start_tick < MAX_NORMAL_TIMELINE_TICKS as i64 {
                    if is_cancelled(cancel) {
                        return Err("Timeleap候補検索がキャンセルされました。".into());
                    }
                    let extension_end_tick = extension_start_tick
                        .saturating_add(GROUP_BOUNDARY_PROBE_CHUNK_TICKS)
                        .min(MAX_NORMAL_TIMELINE_TICKS as i64);
                    let chunk = Frame1CandidateRange::new(extension_start_tick, extension_end_tick);
                    let (extension_candidates, extension_stats, extension_target_reached) =
                        search_consider_delay_candidates_parallel_with_caches(
                            config,
                            delay,
                            chunk.start_tick,
                            chunk.end_tick,
                            extension_status.clone(),
                            extension_cursor,
                            extension_sfmt.clone(),
                            cache,
                            event_generation_bounds,
                            &shared_caches,
                            cancel,
                        )?;
                    stats.boundary_probe_ticks = stats
                        .boundary_probe_ticks
                        .saturating_add(extension_stats.normal_ticks_scanned);
                    stats.add_candidate_stats(extension_stats);
                    let boundary = take_contiguous_prefix(extension_candidates, chunk);
                    stats.boundary_frame1_origin_ticks = stats
                        .boundary_frame1_origin_ticks
                        .saturating_add(boundary.connected_ticks);
                    let reaches_next_chunk = boundary.reaches_chunk_edge;
                    candidates.extend(boundary.candidates);
                    if candidates.len() >= MAX_CANDIDATES {
                        return Err("Timeleap候補が多すぎます。検索範囲を狭めてください。".into());
                    }
                    if !reaches_next_chunk || extension_target_reached {
                        break;
                    }
                    while extension_cursor.tick < extension_end_tick {
                        advance_one_tick_without_blink_list(
                            &mut extension_status,
                            &mut extension_cursor,
                            &mut extension_sfmt,
                        );
                    }
                    extension_start_tick = extension_end_tick;
                }
            }
            #[cfg(test)]
            let grouping_started = std::time::Instant::now();
            let groups = groups_intersecting_tick_window_cancelable(
                candidates, start_tick, end_tick, cancel,
            )?;
            #[cfg(test)]
            {
                stats.candidate_grouping_micros = grouping_started.elapsed().as_micros() as u64;
            }
            return Ok((groups, stats));
        }
    }

    let mut cursor = scope.core_start.cursor;
    let mut status = scope.core_start.status.clone();
    let mut sfmt = cached_sfmt_at(cache, cursor.sfmt_position)?;
    let mut candidates = Vec::new();
    let mut target_suffixes = HashMap::new();
    let mut delay_window_suffixes = HashMap::new();
    let mut delay_input_window_cache = HashMap::new();
    let mut extension_delay_window_suffixes = HashMap::new();
    let mut scan_end_tick = end_tick;
    let max_scanned_candidates = MAX_CANDIDATES;

    // 進行していない状態も含め、各tickの開始状態を候補として評価する。
    while cursor.tick < scan_end_tick {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        if cursor.frame >= config.target_frame {
            break;
        }
        let frame1 = cursor.frame;
        let frame1_tick = cursor.tick;
        let mut candidate_status = status.clone();
        let mut candidate_cursor = cursor;
        let mut candidate_sfmt = sfmt.clone();
        let pre_frame = candidate_cursor.frame;
        let pre_sfmt_position = candidate_cursor.sfmt_position;
        let pre_status = candidate_status.clone();
        apply_event_with_profile(
            &mut candidate_status,
            &mut candidate_cursor,
            &mut candidate_sfmt,
            config.mode,
            &config.courier.generation_profile(),
        );
        advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
        stats.normal_ticks_scanned = stats.normal_ticks_scanned.saturating_add(1);
        if cursor.frame > config.target_frame {
            break;
        }
        stats.candidates_tested += 1;
        let candidate = evaluate_serial_candidate(
            config,
            consider_delay,
            event_generation_bounds,
            cache,
            merge_future_states,
            frame1,
            frame1_tick,
            end_tick,
            pre_frame,
            pre_sfmt_position,
            pre_status,
            candidate_status,
            candidate_cursor,
            candidate_sfmt,
            &mut target_suffixes,
            &mut delay_window_suffixes,
            &mut delay_input_window_cache,
            &mut extension_delay_window_suffixes,
            cancel,
            &mut stats,
        )?;
        let candidate_reached_target = candidate.is_some();
        if let Some(candidate) = candidate {
            candidates.push(candidate);
            if candidates.len() >= max_scanned_candidates {
                return Err("Timeleap候補が多すぎます。検索範囲を狭めてください。".into());
            }
        }
        if frame1_tick >= end_tick {
            stats.boundary_probe_ticks = stats.boundary_probe_ticks.saturating_add(1);
            if !candidate_reached_target {
                break;
            }
            stats.boundary_frame1_origin_ticks =
                stats.boundary_frame1_origin_ticks.saturating_add(1);
            scan_end_tick = scan_end_tick
                .saturating_add(1)
                .min(MAX_NORMAL_TIMELINE_TICKS as i64);
        } else if frame1_tick.saturating_add(1) == end_tick && candidate_reached_target {
            scan_end_tick = scan_end_tick
                .saturating_add(1)
                .min(MAX_NORMAL_TIMELINE_TICKS as i64);
        }
        if frame1_tick < end_tick {
            stats.core_frame1_origin_ticks = stats.core_frame1_origin_ticks.saturating_add(1);
        }
    }

    if candidates
        .iter()
        .any(|candidate| candidate.frame1_tick == start_tick)
        && start_tick > 0
    {
        let mut boundary_end = start_tick;
        let checkpoints = scope
            .lower_checkpoints
            .iter()
            .rev()
            .filter(|checkpoint| checkpoint.cursor.tick < start_tick)
            .cloned()
            .collect::<Vec<_>>();
        for checkpoint in checkpoints {
            if checkpoint.cursor.tick >= boundary_end {
                continue;
            }
            let chunk = Frame1CandidateRange::new(checkpoint.cursor.tick, boundary_end);
            let chunk_candidates = scan_serial_candidate_range(
                config,
                consider_delay,
                event_generation_bounds,
                cache,
                merge_future_states,
                end_tick,
                checkpoint,
                chunk,
                &mut target_suffixes,
                &mut delay_window_suffixes,
                &mut delay_input_window_cache,
                &mut extension_delay_window_suffixes,
                cancel,
                &mut stats,
            )?;
            let boundary = take_contiguous_suffix(chunk_candidates, chunk);
            stats.boundary_frame1_origin_ticks = stats
                .boundary_frame1_origin_ticks
                .saturating_add(boundary.connected_ticks);
            let reaches_prior_checkpoint = boundary.reaches_chunk_edge;
            candidates.extend(boundary.candidates);
            if candidates.len() >= MAX_CANDIDATES {
                return Err("Timeleap候補が多すぎます。検索範囲を狭めてください。".into());
            }
            if !reaches_prior_checkpoint {
                break;
            }
            boundary_end = chunk.start_tick;
        }
    }
    stats.remembered_checkpoints = target_suffixes.len()
        + delay_window_suffixes
            .values()
            .map(HashMap::len)
            .sum::<usize>()
        + extension_delay_window_suffixes
            .values()
            .map(HashMap::len)
            .sum::<usize>();
    #[cfg(test)]
    let grouping_started = std::time::Instant::now();
    let groups =
        groups_intersecting_tick_window_cancelable(candidates, start_tick, end_tick, cancel)?;
    #[cfg(test)]
    {
        stats.candidate_grouping_micros = grouping_started.elapsed().as_micros() as u64;
    }
    Ok((groups, stats))
}

#[allow(clippy::too_many_arguments)]
fn evaluate_serial_candidate(
    config: &TimeleapConfig,
    consider_delay: Option<&ConsiderDelayConfig>,
    event_generation_bounds: EventGenerationFrameBounds,
    cache: &ObservationCache,
    merge_future_states: bool,
    frame1: i64,
    frame1_tick: i64,
    core_end_tick: i64,
    pre_frame: i64,
    pre_sfmt_position: i64,
    pre_status: TimeleapStatus,
    mut candidate_status: TimeleapStatus,
    mut candidate_cursor: Cursor,
    mut candidate_sfmt: Sfmt,
    target_suffixes: &mut HashMap<FutureStateKey, CachedTargetSuffix>,
    delay_window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    delay_input_window_cache: &mut HashMap<
        ConsiderDelayInputWindowKey,
        Vec<CachedConsiderDelayInputHit>,
    >,
    extension_delay_window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Option<TimeleapCandidate>, String> {
    let target_inputs = if let Some(delay) = consider_delay {
        let delay_ticks = i32::try_from(delay.delay_frames / 2 + 2)
            .map_err(|_| "Consider Delayの値が計算可能な範囲を超えています。")?;
        if !merge_future_states {
            return Err("Consider Delay検索では候補Timelineの合流計算が必要です。".into());
        }
        let hits = search_consider_delay_inputs_with_event_bound(
            delay,
            delay_ticks,
            config,
            &candidate_status,
            &candidate_cursor,
            &candidate_sfmt,
            cache,
            if frame1_tick >= core_end_tick {
                extension_delay_window_suffixes
            } else {
                delay_window_suffixes
            },
            delay_input_window_cache,
            event_generation_bounds,
            None,
            None,
            cancel,
            stats,
        )?;
        let Some(representative) = hits.get(hits.len() / 2) else {
            return Ok(None);
        };
        candidate_cursor = representative.landing_cursor;
        candidate_status = representative.landing_status.clone();
        hits.iter()
            .map(|hit| TimeleapTargetInput {
                frame: hit.input_cursor.frame,
                tick: hit.input_cursor.tick,
            })
            .collect::<Vec<_>>()
    } else {
        let reached = if merge_future_states {
            advance_to_target_with_merging(
                config.target_frame,
                &mut candidate_status,
                &mut candidate_cursor,
                &mut candidate_sfmt,
                target_suffixes,
                cancel,
                stats,
            )?
        } else {
            advance_to_target(
                config.target_frame,
                &mut candidate_status,
                &mut candidate_cursor,
                &mut candidate_sfmt,
            )
        };
        if !reached {
            return Ok(None);
        }
        vec![TimeleapTargetInput {
            frame: config.target_frame,
            tick: candidate_cursor.tick,
        }]
    };

    Ok(Some(TimeleapCandidate {
        frame1,
        frame1_tick,
        post_tick: candidate_cursor.tick,
        grace_frames: 0,
        pre_frame,
        pre_sfmt_position,
        pre_status,
        post_frame: candidate_cursor.frame,
        post_sfmt_position: candidate_cursor.sfmt_position,
        post_status: candidate_status,
        target_inputs: target_inputs.into(),
    }))
}

#[allow(clippy::too_many_arguments)]
fn scan_serial_candidate_range(
    config: &TimeleapConfig,
    consider_delay: Option<&ConsiderDelayConfig>,
    event_generation_bounds: EventGenerationFrameBounds,
    cache: &ObservationCache,
    merge_future_states: bool,
    core_end_tick: i64,
    checkpoint: CandidateTimelineCheckpoint,
    range: Frame1CandidateRange,
    target_suffixes: &mut HashMap<FutureStateKey, CachedTargetSuffix>,
    delay_window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    delay_input_window_cache: &mut HashMap<
        ConsiderDelayInputWindowKey,
        Vec<CachedConsiderDelayInputHit>,
    >,
    extension_delay_window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Vec<TimeleapCandidate>, String> {
    let mut cursor = checkpoint.cursor;
    let mut status = checkpoint.status;
    let mut sfmt = cached_sfmt_at(cache, cursor.sfmt_position)?;
    let mut candidates = Vec::new();
    while cursor.tick < range.end_tick {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        if cursor.frame >= config.target_frame {
            break;
        }
        let frame1 = cursor.frame;
        let frame1_tick = cursor.tick;
        let mut candidate_status = status.clone();
        let mut candidate_cursor = cursor;
        let mut candidate_sfmt = sfmt.clone();
        let pre_frame = candidate_cursor.frame;
        let pre_sfmt_position = candidate_cursor.sfmt_position;
        let pre_status = candidate_status.clone();
        apply_event_with_profile(
            &mut candidate_status,
            &mut candidate_cursor,
            &mut candidate_sfmt,
            config.mode,
            &config.courier.generation_profile(),
        );
        advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
        stats.normal_ticks_scanned = stats.normal_ticks_scanned.saturating_add(1);
        stats.boundary_probe_ticks = stats.boundary_probe_ticks.saturating_add(1);
        if cursor.frame > config.target_frame {
            break;
        }
        stats.candidates_tested = stats.candidates_tested.saturating_add(1);
        if let Some(candidate) = evaluate_serial_candidate(
            config,
            consider_delay,
            event_generation_bounds,
            cache,
            merge_future_states,
            frame1,
            frame1_tick,
            core_end_tick,
            pre_frame,
            pre_sfmt_position,
            pre_status,
            candidate_status,
            candidate_cursor,
            candidate_sfmt,
            target_suffixes,
            delay_window_suffixes,
            delay_input_window_cache,
            extension_delay_window_suffixes,
            cancel,
            stats,
        )? {
            candidates.push(candidate);
            if candidates.len() >= MAX_CANDIDATES {
                return Err("Timeleap候補が多すぎます。検索範囲を狭めてください。".into());
            }
        }
    }
    Ok(candidates)
}

#[cfg(test)]
pub(super) fn groups_intersecting_tick_window(
    candidates: Vec<TimeleapCandidate>,
    start_tick: i64,
    end_tick: i64,
) -> Result<Vec<TimeleapCandidateGroup>, String> {
    groups_intersecting_tick_window_cancelable(candidates, start_tick, end_tick, None)
}

fn groups_intersecting_tick_window_cancelable(
    candidates: Vec<TimeleapCandidate>,
    start_tick: i64,
    end_tick: i64,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<TimeleapCandidateGroup>, String> {
    let mut included_candidate_ticks = std::collections::HashSet::new();
    let grouped = group_candidates_cancelable(candidates, cancel)?;
    let mut groups = Vec::with_capacity(grouped.len());
    for (group_index, group) in grouped.into_iter().enumerate() {
        if group_index % CANDIDATE_GROUP_CANCEL_INTERVAL == 0 && is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        let intersects_window =
            group.frame1_tick_start < end_tick && group.frame1_tick_end >= start_tick;
        if intersects_window {
            for (candidate_index, candidate) in group.candidates.iter().enumerate() {
                if candidate_index % CANDIDATE_GROUP_CANCEL_INTERVAL == 0 && is_cancelled(cancel) {
                    return Err("Timeleap候補検索がキャンセルされました。".into());
                }
                included_candidate_ticks.insert(candidate.frame1_tick);
            }
            groups.push(group);
        }
    }
    if included_candidate_ticks.len() >= MAX_CANDIDATES {
        return Err("Timeleap候補が多すぎます。検索範囲を狭めてください。".into());
    }
    Ok(groups)
}

// This cache owner is constructed once per Frame1 search request and never
// escapes it. The request fixes generator, Delay settings, Target, and model
// count, so keys can describe only the exact timeline state/window.
struct ConsiderDelaySearchCaches {
    pool: rayon::ThreadPool,
    transition_values: Option<Arc<[u64]>>,
    transition_trigger_index: Option<NoBlinkTriggerIndex>,
    transition_values_start: i64,
    preparation_budget: Arc<CacheMemoryBudget>,
    preparation_suffixes: SharedTargetSuffixCache,
    consider_delay_input_budget: Arc<CacheMemoryBudget>,
    consider_delay_input_suffixes: SharedConsiderDelayInputSuffixCache,
    consider_delay_input_results: SharedConsiderDelayInputResultCache,
}

impl ConsiderDelaySearchCaches {
    fn new(
        models: usize,
        cache: &ObservationCache,
        cursor: Cursor,
        target_frame: i64,
    ) -> Result<Self, String> {
        let estimated_entry_bytes = estimate_memo_entry_bytes(models);
        let worker_count = rayon::current_num_threads().min(MAX_PARALLEL_TIMELEAP_WORKERS);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count.max(1))
            .build()
            .map_err(|error| format!("Timeleap検索workerを作成できません: {error}"))?;
        let transition_values = candidate_transition_values(cache, cursor, target_frame)?;
        let transition_trigger_index = transition_values.as_deref().map(NoBlinkTriggerIndex::new);
        let preparation_budget = Arc::new(CacheMemoryBudget::new(MAX_TIMELEAP_MEMO_CACHE_BYTES));
        let preparation_suffixes =
            SharedTargetSuffixCache::with_budget(preparation_budget.clone(), estimated_entry_bytes);
        let consider_delay_input_budget = Arc::new(CacheMemoryBudget::new(
            MAX_SHARED_CONSIDER_DELAY_INPUT_CACHE_BYTES,
        ));
        let consider_delay_input_suffixes = SharedConsiderDelayInputSuffixCache::with_budget(
            consider_delay_input_budget.clone(),
            estimated_entry_bytes,
        );
        let consider_delay_input_results = SharedConsiderDelayInputResultCache::with_budget(
            consider_delay_input_budget.clone(),
            estimated_entry_bytes,
        );
        Ok(Self {
            pool,
            transition_values,
            transition_trigger_index,
            transition_values_start: cursor.sfmt_position,
            preparation_budget,
            preparation_suffixes,
            consider_delay_input_budget,
            consider_delay_input_suffixes,
            consider_delay_input_results,
        })
    }
}

#[cfg(test)]
pub(super) fn search_consider_delay_candidates_parallel<B: Into<EventGenerationFrameBounds>>(
    config: &TimeleapConfig,
    delay: &ConsiderDelayConfig,
    start_tick: i64,
    end_tick: i64,
    status: TimeleapStatus,
    cursor: Cursor,
    sfmt: Sfmt,
    cache: &ObservationCache,
    event_generation_bounds: B,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<TimeleapCandidate>, TimeleapSearchStats), String> {
    let event_generation_bounds = event_generation_bounds.into();
    let shared_caches =
        ConsiderDelaySearchCaches::new(config.models, cache, cursor, config.target_frame)?;
    let (candidates, stats, _) = search_consider_delay_candidates_parallel_with_caches(
        config,
        delay,
        start_tick,
        end_tick,
        status,
        cursor,
        sfmt,
        cache,
        event_generation_bounds,
        &shared_caches,
        cancel,
    )?;
    Ok((candidates, stats))
}

fn search_consider_delay_candidates_parallel_with_caches(
    config: &TimeleapConfig,
    delay: &ConsiderDelayConfig,
    start_tick: i64,
    end_tick: i64,
    status: TimeleapStatus,
    cursor: Cursor,
    sfmt: Sfmt,
    cache: &ObservationCache,
    event_generation_bounds: EventGenerationFrameBounds,
    shared_caches: &ConsiderDelaySearchCaches,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<TimeleapCandidate>, TimeleapSearchStats, bool), String> {
    let enumeration_started = std::time::Instant::now();
    let delay_ticks = i32::try_from(delay.delay_frames / 2 + 2)
        .map_err(|_| "Consider Delayの値が計算可能な範囲を超えています。")?;
    let raining = status.raining;
    let window_frames = consider_delay_initial_window_frames(
        delay,
        delay_ticks,
        config,
        event_generation_bounds,
        raining,
    )
    .0;
    let event_input_preroll_frames = if matches!(delay.generator, ConsiderDelayGenerator::Event) {
        let max_frames_per_tick = config.models.max(1).saturating_add(usize::from(raining)) as i64;
        max_frames_per_tick
    } else {
        0
    };
    let transition_values_start = shared_caches.transition_values_start;
    let transition_values = shared_caches.transition_values.as_deref();
    let transition_trigger_index = shared_caches.transition_trigger_index.as_ref();
    let estimated_entry_bytes = estimate_memo_entry_bytes(config.models);
    let preparation_suffix_states_before = shared_caches.preparation_suffixes.len();
    let event_input_suffix_states_before = shared_caches.consider_delay_input_suffixes.len();
    let source_start_tick = cursor.tick;
    let mut source = CandidateSnapshotSource {
        cursor,
        status,
        sfmt,
        start_tick,
        end_tick,
        target_frame: config.target_frame,
        target_reached: false,
        mode: config.mode,
        courier: config.courier.generation_profile(),
    };
    let mut timelines = Vec::<MergedConsiderDelayTimeline>::new();
    let mut timeline_by_state = HashMap::<FutureStateKey, usize>::new();
    let mut stats = TimeleapSearchStats::default();
    loop {
        let mut batch = Vec::with_capacity(CANDIDATE_PREPARATION_BATCH_SIZE);
        #[cfg(test)]
        let snapshot_started = std::time::Instant::now();
        while batch.len() < CANDIDATE_PREPARATION_BATCH_SIZE {
            let Some(input) = source.next(cancel)? else {
                break;
            };
            batch.push(input);
        }
        #[cfg(test)]
        {
            stats.candidate_snapshot_micros = stats
                .candidate_snapshot_micros
                .saturating_add(snapshot_started.elapsed().as_micros() as u64);
        }
        if batch.is_empty() {
            break;
        }
        stats.candidates_tested = stats.candidates_tested.saturating_add(batch.len() as u64);
        stats.post_event_snapshots = stats
            .post_event_snapshots
            .saturating_add(batch.len() as u64);

        #[cfg(test)]
        let preparation_started = std::time::Instant::now();
        let prepared = shared_caches.pool.install(|| {
            batch
                .into_par_iter()
                .map(|input| {
                    prepare_consider_delay_candidate(
                        input,
                        window_frames,
                        event_input_preroll_frames,
                        config,
                        cache,
                        &shared_caches.preparation_suffixes,
                        transition_values.as_deref(),
                        transition_trigger_index,
                        transition_values_start,
                        estimated_entry_bytes,
                        cancel,
                    )
                })
                .collect::<Vec<_>>()
        });
        #[cfg(test)]
        {
            stats.candidate_preparation_wall_micros = stats
                .candidate_preparation_wall_micros
                .saturating_add(preparation_started.elapsed().as_micros() as u64);
        }
        for result in prepared {
            let (key, mut timeline, preparation_stats) = result?;
            stats.candidate_ticks_simulated = stats
                .candidate_ticks_simulated
                .saturating_add(preparation_stats.candidate_ticks_simulated);
            stats.frame1_to_a_ticks_simulated = stats
                .frame1_to_a_ticks_simulated
                .saturating_add(preparation_stats.candidate_ticks_simulated);
            stats.merge_hits = stats
                .merge_hits
                .saturating_add(preparation_stats.merge_hits);
            stats.shared_suffix_cache_hits = stats
                .shared_suffix_cache_hits
                .saturating_add(preparation_stats.shared_suffix_cache_hits);
            if let Some(&index) = timeline_by_state.get(&key) {
                timelines[index].origins.append(&mut timeline.origins);
            } else {
                let index = timelines.len();
                timeline_by_state.insert(key, index);
                timelines.push(timeline);
            }
        }
    }
    let preparation_suffix_states = shared_caches
        .preparation_suffixes
        .len()
        .saturating_sub(preparation_suffix_states_before);
    stats.remembered_checkpoints = preparation_suffix_states;
    stats.shared_suffix_cache_states = preparation_suffix_states;
    stats.shared_cache_peak_bytes = shared_caches.preparation_budget.peak_bytes();

    stats.frame1_enumeration_micros = enumeration_started.elapsed().as_micros() as u64;
    stats.merged_timeline_work_items = timelines.len() as u64;

    // Frame1 candidates are fully enumerated and exact-state convergence is
    // complete before expensive A-input scans are scheduled. Each task owns its
    // mutable Timeline cursor; only immutable memo results are shared.
    let a_input_started = std::time::Instant::now();
    let outputs = shared_caches.pool.install(|| {
        timelines
            .into_par_iter()
            .map(
                |work| -> Result<(Vec<TimeleapCandidate>, TimeleapSearchStats), String> {
                    if is_cancelled(cancel) {
                        return Err("Timeleap候補検索がキャンセルされました。".into());
                    }
                    let mut local_stats = TimeleapSearchStats::default();
                    let candidate_status = work.scan_status;
                    let candidate_cursor = work.scan_cursor;
                    let window_frames = consider_delay_initial_window_frames(
                        delay,
                        delay_ticks,
                        config,
                        event_generation_bounds,
                        candidate_status.raining,
                    )
                    .0;
                    let window_start = config
                        .target_frame
                        .saturating_sub(window_frames)
                        .max(config.start_frame.max(candidate_cursor.frame));
                    let result_key = ConsiderDelayInputWindowKey {
                        window_start,
                        state: exact_future_state_key(candidate_cursor, &candidate_status),
                    };
                    let mut pending_results =
                        PendingSharedMemoPath::<Vec<CachedConsiderDelayInputHit>>::default();
                    let cache_miss = loop {
                        match shared_caches
                            .consider_delay_input_results
                            .lookup_or_reserve(&result_key)
                        {
                            SharedMemoLookup::Ready(cached_hits) => {
                                local_stats.shared_consider_delay_input_result_hits = local_stats
                                    .shared_consider_delay_input_result_hits
                                    .saturating_add(1);
                                let hits = restore_cached_consider_delay_input_hits(
                                    &cached_hits,
                                    &candidate_cursor,
                                );
                                let restore_started = std::time::Instant::now();
                                let candidates = restore_shared_timeline_result(
                                    work.origins,
                                    work.representative_scan_tick,
                                    hits,
                                );
                                local_stats.candidate_restore_micros =
                                    restore_started.elapsed().as_micros() as u64;
                                return Ok((candidates, local_stats));
                            }
                            SharedMemoLookup::Owner(entry) => {
                                local_stats.shared_consider_delay_input_result_misses = local_stats
                                    .shared_consider_delay_input_result_misses
                                    .saturating_add(1);
                                pending_results.claim(entry, candidate_cursor.tick);
                                break true;
                            }
                            SharedMemoLookup::Wait(entry) => {
                                local_stats.shared_consider_delay_input_result_waits = local_stats
                                    .shared_consider_delay_input_result_waits
                                    .saturating_add(1);
                                match shared_caches
                                    .consider_delay_input_results
                                    .wait(&entry, cancel)?
                                {
                                    SharedMemoWait::Ready(cached_hits) => {
                                        local_stats.shared_consider_delay_input_result_hits =
                                            local_stats
                                                .shared_consider_delay_input_result_hits
                                                .saturating_add(1);
                                        let hits = restore_cached_consider_delay_input_hits(
                                            &cached_hits,
                                            &candidate_cursor,
                                        );
                                        let restore_started = std::time::Instant::now();
                                        let candidates = restore_shared_timeline_result(
                                            work.origins,
                                            work.representative_scan_tick,
                                            hits,
                                        );
                                        local_stats.candidate_restore_micros =
                                            restore_started.elapsed().as_micros() as u64;
                                        return Ok((candidates, local_stats));
                                    }
                                    SharedMemoWait::Retry => continue,
                                }
                            }
                            SharedMemoLookup::Uncached => {
                                local_stats.shared_consider_delay_input_result_bypasses =
                                    local_stats
                                        .shared_consider_delay_input_result_bypasses
                                        .saturating_add(1);
                                break false;
                            }
                        }
                    };
                    let mut window_suffixes = HashMap::new();
                    let mut input_window_cache = HashMap::new();
                    let candidate_sfmt = cached_sfmt_at(cache, candidate_cursor.sfmt_position)?;
                    let hits = if let Some(values) = transition_values.as_deref() {
                        search_consider_delay_inputs_with_event_bound_and_values(
                            delay,
                            delay_ticks,
                            config,
                            &candidate_status,
                            &candidate_cursor,
                            &candidate_sfmt,
                            cache,
                            &mut window_suffixes,
                            &mut input_window_cache,
                            event_generation_bounds,
                            None,
                            Some(&shared_caches.consider_delay_input_suffixes),
                            transition_values_start,
                            values,
                            cancel,
                            &mut local_stats,
                        )?
                    } else {
                        search_consider_delay_inputs_with_event_bound(
                            delay,
                            delay_ticks,
                            config,
                            &candidate_status,
                            &candidate_cursor,
                            &candidate_sfmt,
                            cache,
                            &mut window_suffixes,
                            &mut input_window_cache,
                            event_generation_bounds,
                            None,
                            Some(&shared_caches.consider_delay_input_suffixes),
                            cancel,
                            &mut local_stats,
                        )?
                    };
                    if cache_miss && hits.len() <= MAX_MEMOIZED_CONSIDER_DELAY_WINDOW_HITS {
                        let cached_hits =
                            cache_consider_delay_input_hits(&hits, candidate_cursor.tick);
                        pending_results.complete_with(|_| cached_hits.clone());
                    }
                    local_stats.remembered_checkpoints =
                        window_suffixes.values().map(HashMap::len).sum::<usize>();
                    let restore_started = std::time::Instant::now();
                    let candidates = restore_shared_timeline_result(
                        work.origins,
                        work.representative_scan_tick,
                        hits,
                    );
                    local_stats.candidate_restore_micros =
                        restore_started.elapsed().as_micros() as u64;
                    Ok((candidates, local_stats))
                },
            )
            .collect::<Vec<_>>()
    });

    stats.a_input_search_micros = a_input_started.elapsed().as_micros() as u64;
    let mut candidates = Vec::new();
    for output in outputs {
        let (mut worker_candidates, worker_stats) = output?;
        stats.add_candidate_stats(worker_stats);
        candidates.append(&mut worker_candidates);
        if candidates.len() >= MAX_CANDIDATES {
            return Err("Timeleap候補が多すぎます。検索範囲を狭めてください。".into());
        }
    }
    if is_cancelled(cancel) {
        return Err("Timeleap候補検索がキャンセルされました。".into());
    }
    stats.a_window_ticks_simulated = stats
        .candidate_ticks_simulated
        .saturating_sub(stats.frame1_to_a_ticks_simulated);
    stats.normal_ticks_scanned = source.cursor.tick.saturating_sub(source_start_tick).max(0) as u64;
    stats.shared_consider_delay_input_suffix_states = shared_caches
        .consider_delay_input_suffixes
        .len()
        .saturating_sub(event_input_suffix_states_before);
    stats.shared_consider_delay_input_suffix_peak_bytes =
        shared_caches.consider_delay_input_budget.peak_bytes();
    Ok((candidates, stats, source.target_reached))
}

fn candidate_transition_values(
    cache: &ObservationCache,
    cursor: Cursor,
    target_frame: i64,
) -> Result<Option<Arc<[u64]>>, String> {
    let Some(distance) = target_frame
        .saturating_sub(cursor.frame)
        .max(0)
        .checked_add(CACHED_CANDIDATE_RNG_TAIL_VALUES)
    else {
        return Ok(None);
    };
    let Ok(count) = usize::try_from(distance) else {
        return Ok(None);
    };
    if cursor.sfmt_position < 0 || count == 0 || count > MAX_CACHED_CANDIDATE_RNG_VALUES {
        return Ok(None);
    }
    let end_exclusive = cursor.sfmt_position.saturating_add(distance);
    if end_exclusive.saturating_sub(1) > cache.available_end() {
        return Ok(None);
    }
    cache
        .read_values(cursor.sfmt_position, count)
        .map(|values| Some(Arc::<[u64]>::from(values)))
        .map_err(|error| format!("候補Timeline用SFMT値を読み込めませんでした: {error}"))
}

fn prepare_consider_delay_candidate(
    input: ConsiderDelayCandidateInput,
    window_frames: i64,
    event_input_preroll_frames: i64,
    config: &TimeleapConfig,
    cache: &ObservationCache,
    shared_suffixes: &SharedTargetSuffixCache,
    transition_values: Option<&[u64]>,
    transition_trigger_index: Option<&NoBlinkTriggerIndex>,
    transition_values_start: i64,
    estimated_entry_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<
    (
        FutureStateKey,
        MergedConsiderDelayTimeline,
        TimeleapSearchStats,
    ),
    String,
> {
    let mut scan_status = input.post_event_status;
    let mut scan_cursor = input.post_event_cursor;
    let mut scan_sfmt = if transition_values.is_some() {
        // Only read this placeholder if the bounded cached-values window ends.
        Sfmt::new(0)
    } else {
        cached_sfmt_at(cache, scan_cursor.sfmt_position)?
    };
    let minimum_frame = config.start_frame.max(scan_cursor.frame);
    let window_start = config
        .target_frame
        .saturating_sub(window_frames)
        .max(minimum_frame);
    // Event7 accepts A input on any displayed Frame within a normal Timeline
    // tick. Stop before the lower bound so the scanner can retain its first
    // in-tick Frame instead of rounding the preparation cursor past it.
    let preparation_target = window_start
        .saturating_sub(event_input_preroll_frames)
        .max(minimum_frame);
    let mut preparation_stats = TimeleapSearchStats::default();
    let suffix_cache_limit = MAX_TIMELEAP_MEMO_CACHE_BYTES / estimated_entry_bytes.max(1);
    if let Some(values) = transition_values {
        advance_to_target_with_cached_values_and_merging(
            preparation_target,
            &mut scan_status,
            &mut scan_cursor,
            &mut scan_sfmt,
            None,
            Some(shared_suffixes),
            cancel,
            &mut preparation_stats,
            suffix_cache_limit,
            transition_values_start,
            values,
            cache,
            transition_trigger_index,
        )?;
    } else {
        advance_to_target_with_merging_with_limit(
            preparation_target,
            &mut scan_status,
            &mut scan_cursor,
            &mut scan_sfmt,
            None,
            Some(shared_suffixes),
            cancel,
            &mut preparation_stats,
            suffix_cache_limit,
        )?;
    }
    let key = exact_future_state_key(scan_cursor, &scan_status);
    let origin = Frame1CandidateOrigin {
        frame1: input.frame1,
        frame1_tick: input.frame1_tick,
        pre_frame: input.pre_frame,
        pre_sfmt_position: input.pre_sfmt_position,
        pre_status: input.pre_status,
        scan_start_tick: scan_cursor.tick,
    };
    Ok((
        key,
        MergedConsiderDelayTimeline {
            representative_scan_tick: scan_cursor.tick,
            scan_status,
            scan_cursor,
            origins: vec![origin],
        },
        preparation_stats,
    ))
}

#[cfg(test)]
pub(super) fn search_candidates_from_cache_unmerged(
    config: &TimeleapConfig,
    cache: &ObservationCache,
) -> Result<(Vec<TimeleapCandidateGroup>, TimeleapSearchStats), String> {
    search_candidates_from_cache_with_stats(config, cache, None, false)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FutureStateKey {
    frame: i64,
    sfmt_position: i64,
    status: TimeleapStatus,
}

#[derive(Clone)]
pub(super) struct CachedTargetSuffix {
    ticks: i64,
    reached: bool,
    terminal_frame: i64,
    terminal_sfmt_position: i64,
    terminal_status: Arc<TimeleapStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg(test)]
pub(super) struct ConsiderDelayScanKey {
    window_start: i64,
    state: FutureStateKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct TargetSuffixKey {
    target: i64,
    state: FutureStateKey,
}

#[derive(Clone)]
#[cfg(test)]
pub(super) struct CachedConsiderDelayHit {
    input_frame: i64,
    input_sfmt_position: i64,
    input_tick_offset: i64,
    landing_frame: i64,
    landing_sfmt_position: i64,
    landing_tick_offset: i64,
    landing_status: TimeleapStatus,
}

pub(super) enum SharedEntryState<V> {
    Pending,
    Ready(V),
    Abandoned,
}

pub(super) struct SharedEntry<V> {
    state: Mutex<SharedEntryState<V>>,
    ready: Condvar,
}

pub(super) struct CacheMemoryBudget {
    limit_bytes: usize,
    retained_bytes: AtomicUsize,
    peak_bytes: AtomicUsize,
}

impl CacheMemoryBudget {
    fn new(limit_bytes: usize) -> Self {
        Self {
            limit_bytes,
            retained_bytes: AtomicUsize::new(0),
            peak_bytes: AtomicUsize::new(0),
        }
    }

    fn reserve(&self, bytes: usize) -> bool {
        let reserved = self
            .retained_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= self.limit_bytes)
            })
            .is_ok();
        if reserved {
            let current = self.retained_bytes.load(Ordering::Acquire);
            self.peak_bytes.fetch_max(current, Ordering::AcqRel);
        }
        reserved
    }

    fn release(&self, bytes: usize) {
        self.retained_bytes.fetch_sub(bytes, Ordering::AcqRel);
    }

    fn peak_bytes(&self) -> usize {
        self.peak_bytes.load(Ordering::Acquire)
    }
}

pub(super) fn estimate_memo_entry_bytes(models: usize) -> usize {
    // Include both state vectors (map key and cached result), hash-table/Arc/
    // synchronization overhead, allocator slack, and a conservative per-model
    // allowance. This is admission control, not an RSS claim.
    512usize.saturating_add(models.max(1).saturating_mul(32))
}

pub(super) enum SharedMemoLookup<V> {
    Ready(V),
    Owner(Arc<SharedEntry<V>>),
    Wait(Arc<SharedEntry<V>>),
    Uncached,
}

pub(super) enum SharedMemoWait<V> {
    Ready(V),
    Retry,
}

pub(super) struct SharedSingleFlightCache<K, V> {
    entries: RwLock<HashMap<K, Arc<SharedEntry<V>>>>,
    entry_limit: usize,
    memory_budget: Option<Arc<CacheMemoryBudget>>,
    estimated_entry_bytes: usize,
    wait_count: AtomicU64,
    hit_count: AtomicU64,
    miss_count: AtomicU64,
    bypass_count: AtomicU64,
}

impl<K, V> SharedSingleFlightCache<K, V>
where
    K: Clone + Eq + Hash + Send + Sync,
    V: Clone + Send + Sync,
{
    #[cfg(test)]
    fn new(entry_limit: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            entry_limit,
            memory_budget: None,
            estimated_entry_bytes: 0,
            wait_count: AtomicU64::new(0),
            hit_count: AtomicU64::new(0),
            miss_count: AtomicU64::new(0),
            bypass_count: AtomicU64::new(0),
        }
    }

    fn with_budget(memory_budget: Arc<CacheMemoryBudget>, estimated_entry_bytes: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            entry_limit: usize::MAX,
            memory_budget: Some(memory_budget),
            estimated_entry_bytes,
            wait_count: AtomicU64::new(0),
            hit_count: AtomicU64::new(0),
            miss_count: AtomicU64::new(0),
            bypass_count: AtomicU64::new(0),
        }
    }

    fn lookup_or_reserve(&self, key: &K) -> SharedMemoLookup<V> {
        loop {
            let existing = self
                .entries
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(key)
                .cloned();
            if let Some(entry) = existing {
                let state = entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match &*state {
                    SharedEntryState::Ready(result) => {
                        self.hit_count.fetch_add(1, Ordering::Relaxed);
                        return SharedMemoLookup::Ready(result.clone());
                    }
                    SharedEntryState::Pending => {
                        return SharedMemoLookup::Wait(entry.clone());
                    }
                    SharedEntryState::Abandoned => {
                        drop(state);
                        let mut entries = self
                            .entries
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if entries
                            .get(key)
                            .is_some_and(|current| Arc::ptr_eq(current, &entry))
                        {
                            entries.remove(key);
                            if let Some(budget) = &self.memory_budget {
                                budget.release(self.estimated_entry_bytes);
                            }
                        }
                    }
                }
                continue;
            }

            let mut entries = self
                .entries
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if entries.contains_key(key) {
                continue;
            }
            if entries.len() >= self.entry_limit
                || self
                    .memory_budget
                    .as_ref()
                    .is_some_and(|budget| !budget.reserve(self.estimated_entry_bytes))
            {
                self.miss_count.fetch_add(1, Ordering::Relaxed);
                self.bypass_count.fetch_add(1, Ordering::Relaxed);
                return SharedMemoLookup::Uncached;
            }
            self.miss_count.fetch_add(1, Ordering::Relaxed);
            let entry = Arc::new(SharedEntry {
                state: Mutex::new(SharedEntryState::Pending),
                ready: Condvar::new(),
            });
            entries.insert(key.clone(), entry.clone());
            return SharedMemoLookup::Owner(entry);
        }
    }

    fn wait(
        &self,
        entry: &SharedEntry<V>,
        cancel: Option<&AtomicBool>,
    ) -> Result<SharedMemoWait<V>, String> {
        self.wait_count.fetch_add(1, Ordering::Relaxed);
        let mut state = entry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match &*state {
                SharedEntryState::Ready(result) => {
                    return Ok(SharedMemoWait::Ready(result.clone()));
                }
                SharedEntryState::Abandoned => return Ok(SharedMemoWait::Retry),
                SharedEntryState::Pending if is_cancelled(cancel) => {
                    return Err("Timeleap候補検索がキャンセルされました。".into());
                }
                SharedEntryState::Pending => {
                    state = entry
                        .ready
                        .wait_timeout(state, Duration::from_millis(10))
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0;
                }
            }
        }
    }

    fn len(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    #[cfg(test)]
    fn wait_count(&self) -> u64 {
        self.wait_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn hit_count(&self) -> u64 {
        self.hit_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn miss_count(&self) -> u64 {
        self.miss_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn bypass_count(&self) -> u64 {
        self.bypass_count.load(Ordering::Relaxed)
    }
}

pub(super) struct PendingSharedMemoPath<V> {
    entries: Vec<(Arc<SharedEntry<V>>, i64)>,
    completed: bool,
}

impl<V> Default for PendingSharedMemoPath<V> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            completed: false,
        }
    }
}

impl<V> PendingSharedMemoPath<V> {
    fn claim(&mut self, entry: Arc<SharedEntry<V>>, tick: i64) {
        self.entries.push((entry, tick));
    }

    fn complete_with(&mut self, result: impl Fn(i64) -> V) {
        for (entry, tick) in &self.entries {
            let mut state = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*state, SharedEntryState::Pending) {
                *state = SharedEntryState::Ready(result(*tick));
                entry.ready.notify_all();
            }
        }
        self.completed = true;
    }
}

impl<V> Drop for PendingSharedMemoPath<V> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        for (entry, _) in &self.entries {
            let mut state = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*state, SharedEntryState::Pending) {
                *state = SharedEntryState::Abandoned;
                entry.ready.notify_all();
            }
        }
    }
}

#[cfg(test)]
pub(super) type SharedConsiderDelayScanCache =
    SharedSingleFlightCache<ConsiderDelayScanKey, Option<CachedConsiderDelayHit>>;
type SharedConsiderDelayInputSuffixCache =
    SharedSingleFlightCache<ConsiderDelayInputWindowKey, CachedConsiderDelayInputSuffix>;
type SharedConsiderDelayInputResultCache =
    SharedSingleFlightCache<ConsiderDelayInputWindowKey, Vec<CachedConsiderDelayInputHit>>;
pub(super) type SharedTargetSuffixCache =
    SharedSingleFlightCache<TargetSuffixKey, CachedTargetSuffix>;
#[cfg(test)]
pub(super) type PendingSharedScanPath = PendingSharedMemoPath<Option<CachedConsiderDelayHit>>;
pub(super) type PendingSharedSuffixPath = PendingSharedMemoPath<CachedTargetSuffix>;

/// Candidate event branches are deterministic once their complete state and absolute
/// SFMT cursor match. Sparse SFMT-position checkpoints let branches reuse the remaining
/// path without storing a key for every cooldown tick.
pub(super) fn advance_to_target_with_merging(
    target: i64,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    suffixes: &mut HashMap<FutureStateKey, CachedTargetSuffix>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<bool, String> {
    advance_to_target_with_merging_with_limit(
        target,
        status,
        cursor,
        sfmt,
        Some(suffixes),
        None,
        cancel,
        stats,
        usize::MAX,
    )
}

pub(super) fn advance_to_target_with_merging_with_limit(
    target: i64,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    suffixes: Option<&mut HashMap<FutureStateKey, CachedTargetSuffix>>,
    shared_suffixes: Option<&SharedTargetSuffixCache>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
    suffix_cache_limit: usize,
) -> Result<bool, String> {
    advance_to_target_with_merging_using(
        target,
        status,
        cursor,
        sfmt,
        suffixes,
        shared_suffixes,
        cancel,
        stats,
        suffix_cache_limit,
        |status, cursor, sfmt| {
            advance_one_tick_without_blink_list(status, cursor, sfmt);
            Ok(())
        },
        |_, _, _, _| 0,
    )
}

#[allow(clippy::too_many_arguments)]
fn advance_to_target_with_cached_values_and_merging(
    target: i64,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    suffixes: Option<&mut HashMap<FutureStateKey, CachedTargetSuffix>>,
    shared_suffixes: Option<&SharedTargetSuffixCache>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
    suffix_cache_limit: usize,
    values_start: i64,
    values: &[u64],
    cache: &ObservationCache,
    transition_trigger_index: Option<&NoBlinkTriggerIndex>,
) -> Result<bool, String> {
    let mut using_cached_values = true;
    advance_to_target_with_merging_using(
        target,
        status,
        cursor,
        sfmt,
        suffixes,
        shared_suffixes,
        cancel,
        stats,
        suffix_cache_limit,
        |status, cursor, sfmt| {
            let active_models = status.model.remain.len();
            if !using_cached_values
                || !advance_one_tick_state_only_from_values(
                    status,
                    cursor,
                    active_models,
                    true,
                    values_start,
                    values,
                )
            {
                *sfmt = cached_sfmt_at(cache, cursor.sfmt_position)?;
                advance_one_tick_without_blink_list(status, cursor, sfmt);
                using_cached_values = false;
            }
            Ok(())
        },
        |status, cursor, target, remaining| {
            advance_no_blink_run_from_values_with_index(
                status,
                cursor,
                target,
                remaining,
                values_start,
                values,
                true,
                transition_trigger_index,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn advance_to_target_with_merging_using<F, G>(
    target: i64,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    suffixes: Option<&mut HashMap<FutureStateKey, CachedTargetSuffix>>,
    shared_suffixes: Option<&SharedTargetSuffixCache>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
    suffix_cache_limit: usize,
    mut advance_tick: F,
    mut fast_forward: G,
) -> Result<bool, String>
where
    F: FnMut(&mut TimeleapStatus, &mut Cursor, &mut Sfmt) -> Result<(), String>,
    G: FnMut(&mut TimeleapStatus, &mut Cursor, i64, usize) -> usize,
{
    let mut checkpoint_path = Vec::new();
    let mut suffixes = suffixes;
    let mut shared_path = PendingSharedSuffixPath::default();
    let mut logical_ticks = 0usize;

    while logical_ticks < MAX_CANDIDATE_REPLAY_TICKS {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }

        if cursor.frame == target || cursor.frame > target {
            let reached = cursor.frame == target;
            if let Some(suffixes) = suffixes.as_deref_mut() {
                remember_target_suffixes(
                    suffixes,
                    checkpoint_path,
                    cursor,
                    status,
                    reached,
                    suffix_cache_limit,
                );
            }
            complete_shared_target_suffixes(&mut shared_path, cursor, status, reached);
            return Ok(reached);
        }

        if is_merge_checkpoint(cursor.sfmt_position) {
            let state = exact_future_state_key(*cursor, status);
            let local_suffix = suffixes
                .as_deref()
                .and_then(|suffixes| suffixes.get(&state).cloned());
            let mut shared_suffix = None;
            if local_suffix.is_none() {
                if let Some(shared_suffixes) = shared_suffixes {
                    let key = TargetSuffixKey {
                        target,
                        state: state.clone(),
                    };
                    loop {
                        match shared_suffixes.lookup_or_reserve(&key) {
                            SharedMemoLookup::Ready(suffix) => {
                                shared_suffix = Some(suffix);
                                stats.shared_suffix_cache_hits =
                                    stats.shared_suffix_cache_hits.saturating_add(1);
                                break;
                            }
                            SharedMemoLookup::Owner(entry) => {
                                shared_path.claim(entry, cursor.tick);
                                break;
                            }
                            SharedMemoLookup::Wait(entry) => {
                                stats.shared_suffix_cache_waits =
                                    stats.shared_suffix_cache_waits.saturating_add(1);
                                match shared_suffixes.wait(&entry, cancel)? {
                                    SharedMemoWait::Ready(suffix) => {
                                        shared_suffix = Some(suffix);
                                        stats.shared_suffix_cache_hits =
                                            stats.shared_suffix_cache_hits.saturating_add(1);
                                        break;
                                    }
                                    SharedMemoWait::Retry => continue,
                                }
                            }
                            SharedMemoLookup::Uncached => break,
                        }
                    }
                }
            }
            if let Some(suffix) = local_suffix.or(shared_suffix) {
                let remaining_ticks = MAX_CANDIDATE_REPLAY_TICKS - logical_ticks;
                if suffix.ticks > 0
                    && usize::try_from(suffix.ticks).is_ok_and(|ticks| ticks <= remaining_ticks)
                {
                    stats.merge_hits += 1;
                    if suffixes.is_some() && checkpoint_path.len() < suffix_cache_limit {
                        checkpoint_path.push((state, cursor.tick));
                    }
                    cursor.frame = suffix.terminal_frame;
                    cursor.sfmt_position = suffix.terminal_sfmt_position;
                    cursor.tick = cursor.tick.saturating_add(suffix.ticks);
                    *status = (*suffix.terminal_status).clone();
                    if let Some(suffixes) = suffixes.as_deref_mut() {
                        remember_target_suffixes(
                            suffixes,
                            checkpoint_path,
                            cursor,
                            status,
                            suffix.reached,
                            suffix_cache_limit,
                        );
                    }
                    complete_shared_target_suffixes(
                        &mut shared_path,
                        cursor,
                        status,
                        suffix.reached,
                    );
                    return Ok(suffix.reached);
                }
            } else {
                if suffixes.is_some()
                    && shared_suffixes.is_none()
                    && checkpoint_path.len() < suffix_cache_limit
                {
                    checkpoint_path.push((state, cursor.tick));
                }
            }
        }

        let idle_ticks = idle_tick_run(status, MAX_CANDIDATE_REPLAY_TICKS - logical_ticks);
        if idle_ticks > 0 {
            advance_idle_ticks(status, cursor, idle_ticks);
            logical_ticks = logical_ticks.saturating_add(idle_ticks);
            continue;
        }

        let fast_forwarded = fast_forward(
            status,
            cursor,
            target,
            MAX_CANDIDATE_REPLAY_TICKS - logical_ticks,
        );
        if fast_forwarded > 0 {
            logical_ticks = logical_ticks.saturating_add(fast_forwarded);
            continue;
        }

        advance_tick(status, cursor, sfmt)?;
        stats.candidate_ticks_simulated += 1;
        logical_ticks = logical_ticks.saturating_add(1);
    }

    let reached = cursor.frame == target;
    if let Some(suffixes) = suffixes.as_deref_mut() {
        remember_target_suffixes(
            suffixes,
            checkpoint_path,
            cursor,
            status,
            reached,
            suffix_cache_limit,
        );
    }
    complete_shared_target_suffixes(&mut shared_path, cursor, status, reached);
    Ok(reached)
}

fn idle_tick_run(status: &TimeleapStatus, remaining_limit: usize) -> usize {
    if status.raining
        || status.model.remain.iter().any(|remain| *remain <= 1)
        || status.fidget_cd == 1
    {
        return 0;
    }

    let mut ticks = remaining_limit;
    for remain in &status.model.remain {
        ticks = ticks.min(usize::try_from(remain.saturating_sub(1)).unwrap_or(0));
    }
    if status.fidget_cd > 1 {
        ticks = ticks.min(usize::try_from(status.fidget_cd - 1).unwrap_or(0));
    }
    ticks
}

fn advance_idle_ticks(status: &mut TimeleapStatus, cursor: &mut Cursor, ticks: usize) {
    let decrement = i32::try_from(ticks).expect("bounded idle tick count fits i32");
    for remain in &mut status.model.remain {
        *remain -= decrement;
    }
    if status.fidget_cd > 1 {
        status.fidget_cd -= decrement;
    }
    cursor.tick = cursor.tick.saturating_add(ticks as i64);
}

pub(super) fn complete_shared_target_suffixes(
    path: &mut PendingSharedSuffixPath,
    terminal: &Cursor,
    terminal_status: &TimeleapStatus,
    reached: bool,
) {
    let terminal = *terminal;
    let terminal_status = Arc::new(terminal_status.clone());
    path.complete_with(|tick| CachedTargetSuffix {
        ticks: terminal.tick.saturating_sub(tick),
        reached,
        terminal_frame: terminal.frame,
        terminal_sfmt_position: terminal.sfmt_position,
        terminal_status: terminal_status.clone(),
    });
}

pub(super) fn is_merge_checkpoint(sfmt_position: i64) -> bool {
    sfmt_position.rem_euclid(MERGE_CHECK_SFMT_INTERVAL) == 0
}

#[cfg(test)]
pub(super) fn future_state_key(cursor: Cursor, status: &TimeleapStatus) -> FutureStateKey {
    // remain=0/1は、同じSFMT位置から同じ瞬き判定へ進む等価な待機状態。
    // 共有・キャッシュのキーだけを正規化し、実際に進めるstatusは変更しない。
    let mut equivalent_status = status.clone();
    for remain in &mut equivalent_status.model.remain {
        if *remain == 1 {
            *remain = 0;
        }
    }
    FutureStateKey {
        frame: cursor.frame,
        sfmt_position: cursor.sfmt_position,
        status: equivalent_status,
    }
}

/// Build a cache key that can safely restore the exact terminal Timeline state.
/// remain=0 and remain=1 have identical future RNG consumption, but their
/// concrete values must not be substituted when a cached status is returned.
pub(super) fn exact_future_state_key(cursor: Cursor, status: &TimeleapStatus) -> FutureStateKey {
    FutureStateKey {
        frame: cursor.frame,
        sfmt_position: cursor.sfmt_position,
        status: status.clone(),
    }
}

pub(super) fn remember_target_suffixes(
    suffixes: &mut HashMap<FutureStateKey, CachedTargetSuffix>,
    checkpoint_path: Vec<(FutureStateKey, i64)>,
    terminal: &Cursor,
    terminal_status: &TimeleapStatus,
    reached: bool,
    cache_limit: usize,
) {
    let terminal_status = Arc::new(terminal_status.clone());
    for (key, tick) in checkpoint_path {
        if suffixes.len() >= cache_limit && !suffixes.contains_key(&key) {
            break;
        }
        suffixes.entry(key).or_insert_with(|| CachedTargetSuffix {
            ticks: terminal.tick.saturating_sub(tick),
            reached,
            terminal_frame: terminal.frame,
            terminal_sfmt_position: terminal.sfmt_position,
            terminal_status: terminal_status.clone(),
        });
    }
}

pub(super) fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|token| token.load(Ordering::Acquire))
}

#[cfg(test)]
pub(super) fn group_candidates(candidates: Vec<TimeleapCandidate>) -> Vec<TimeleapCandidateGroup> {
    group_candidates_cancelable(candidates, None)
        .expect("candidate grouping without a cancellation token cannot be cancelled")
}

fn group_candidates_cancelable(
    mut candidates: Vec<TimeleapCandidate>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<TimeleapCandidateGroup>, String> {
    let mut needs_sort = false;
    for (index, pair) in candidates.windows(2).enumerate() {
        if index % CANDIDATE_GROUP_CANCEL_INTERVAL == 0 && is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        if pair[0].frame1_tick > pair[1].frame1_tick {
            needs_sort = true;
            break;
        }
    }
    if needs_sort {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        candidates.sort_by_key(|candidate| candidate.frame1_tick);
    }

    let mut groups: Vec<TimeleapCandidateGroup> = Vec::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        if index % CANDIDATE_GROUP_CANCEL_INTERVAL == 0 && is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        if let Some(group) = groups.last_mut() {
            let expected_next = group.frame1_tick_end.saturating_add(1);
            if candidate.frame1_tick <= expected_next {
                group.frame1_end = candidate.frame1;
                group.frame1_tick_end = candidate.frame1_tick;
                group.candidates.push(candidate);
                group.grace_frames = group
                    .frame1_tick_end
                    .saturating_sub(group.frame1_tick_start)
                    .saturating_add(1)
                    .saturating_mul(2);
                group.representative_frame1 = representative_candidate_frame(group);
                group.frame2_ticks = representative_frame2_ticks(group);
                continue;
            }
        }
        let frame1 = candidate.frame1;
        let frame1_tick = candidate.frame1_tick;
        groups.push(TimeleapCandidateGroup {
            frame1_start: frame1,
            frame1_end: frame1,
            frame1_tick_start: frame1_tick,
            frame1_tick_end: frame1_tick,
            representative_frame1: frame1,
            frame2_ticks: candidate.frame1_to_frame2_ticks(),
            grace_frames: 2,
            target_frame_start: 0,
            target_frame_end: 0,
            target_tick_start: 0,
            target_tick_end: 0,
            representative_target_frame: 0,
            target_grace_frames: 0,
            candidates: smallvec::smallvec![candidate],
        });
    }
    let candidate_count = groups
        .iter()
        .map(|group| group.candidates.len())
        .sum::<usize>();
    let mut expanded = Vec::with_capacity(candidate_count.saturating_mul(2));
    for group in groups {
        let mut remaining = group.candidates.into_iter().peekable();
        let mut run = Vec::new();
        while let Some(first) = remaining.next() {
            run.clear();
            run.push(first);
            loop {
                if run.len() % CANDIDATE_GROUP_CANCEL_INTERVAL == 0 && is_cancelled(cancel) {
                    return Err("Timeleap候補検索がキャンセルされました。".into());
                }
                let expected_next = run
                    .last()
                    .expect("a Frame1 run always has its first candidate")
                    .frame1_tick
                    .saturating_add(1);
                if !remaining
                    .peek()
                    .is_some_and(|candidate| candidate.frame1_tick == expected_next)
                {
                    break;
                }
                run.push(
                    remaining
                        .next()
                        .expect("peeked candidate remains in the iterator"),
                );
            }

            let frame1_tick_start = run[0].frame1_tick;
            let frame1_tick_end = run[run.len() - 1].frame1_tick;
            let frame1_start = run[0].frame1;
            let frame1_end = run[run.len() - 1].frame1;
            let grace_frames = frame1_tick_end
                .saturating_sub(frame1_tick_start)
                .saturating_add(1)
                .saturating_mul(2);

            for (candidate_index, candidate) in run.drain(..).enumerate() {
                if candidate_index % CANDIDATE_GROUP_CANCEL_INTERVAL == 0 && is_cancelled(cancel) {
                    return Err("Timeleap候補検索がキャンセルされました。".into());
                }
                let mut candidate = Some(candidate);
                let target_inputs_are_not_strictly_sorted = candidate
                    .as_ref()
                    .expect("candidate is present before grouping")
                    .target_inputs
                    .windows(2)
                    .any(|pair| pair[0].tick >= pair[1].tick);
                if target_inputs_are_not_strictly_sorted {
                    let intervals = target_input_intervals(
                        candidate
                            .as_ref()
                            .expect("candidate is present before fallback grouping"),
                    );
                    let interval_count = intervals.len();
                    for (interval_index, interval) in intervals.into_iter().enumerate() {
                        let interval_start = interval[0];
                        let interval_end = interval[interval.len() - 1];
                        let representative = interval[interval.len() / 2];
                        let grouped_candidate = if interval_index + 1 == interval_count {
                            candidate.take().expect("last interval owns the candidate")
                        } else {
                            candidate
                                .as_ref()
                                .expect("candidate remains until its last interval")
                                .clone()
                        };
                        push_candidate_target_group(
                            &mut expanded,
                            frame1_start,
                            frame1_end,
                            frame1_tick_start,
                            frame1_tick_end,
                            grace_frames,
                            grouped_candidate,
                            interval_start,
                            interval_end,
                            representative,
                            interval.len(),
                        );
                    }
                } else if candidate
                    .as_ref()
                    .expect("candidate is present before empty-input grouping")
                    .target_inputs
                    .is_empty()
                {
                    let candidate = candidate.take().expect("candidate is present");
                    let target = TimeleapTargetInput {
                        frame: candidate.post_frame,
                        tick: candidate.post_tick,
                    };
                    push_candidate_target_group(
                        &mut expanded,
                        frame1_start,
                        frame1_end,
                        frame1_tick_start,
                        frame1_tick_end,
                        grace_frames,
                        candidate,
                        target,
                        target,
                        target,
                        1,
                    );
                } else {
                    let target_input_len = candidate
                        .as_ref()
                        .expect("candidate is present before interval grouping")
                        .target_inputs
                        .len();
                    let mut start = 0;
                    loop {
                        let (end, interval_start, interval_end, representative, is_last) = {
                            let inputs = &candidate
                                .as_ref()
                                .expect("candidate remains until its last interval")
                                .target_inputs;
                            let mut end = start + 1;
                            while end < target_input_len
                                && inputs[end].tick == inputs[end - 1].tick.saturating_add(1)
                            {
                                end += 1;
                            }
                            let interval = &inputs[start..end];
                            (
                                end,
                                interval[0],
                                interval[interval.len() - 1],
                                interval[interval.len() / 2],
                                end == target_input_len,
                            )
                        };
                        let grouped_candidate = if is_last {
                            candidate.take().expect("last interval owns the candidate")
                        } else {
                            candidate
                                .as_ref()
                                .expect("candidate remains until its last interval")
                                .clone()
                        };
                        push_candidate_target_group(
                            &mut expanded,
                            frame1_start,
                            frame1_end,
                            frame1_tick_start,
                            frame1_tick_end,
                            grace_frames,
                            grouped_candidate,
                            interval_start,
                            interval_end,
                            representative,
                            end - start,
                        );
                        if is_last {
                            break;
                        }
                        start = end;
                    }
                }
            }
        }
    }
    Ok(expanded)
}

#[allow(clippy::too_many_arguments)]
fn push_candidate_target_group(
    expanded: &mut Vec<TimeleapCandidateGroup>,
    frame1_start: i64,
    frame1_end: i64,
    frame1_tick_start: i64,
    frame1_tick_end: i64,
    grace_frames: i64,
    candidate: TimeleapCandidate,
    target_start: TimeleapTargetInput,
    target_end: TimeleapTargetInput,
    target_representative: TimeleapTargetInput,
    interval_len: usize,
) {
    expanded.push(TimeleapCandidateGroup {
        frame1_start,
        frame1_end,
        frame1_tick_start,
        frame1_tick_end,
        representative_frame1: candidate.frame1,
        frame2_ticks: target_representative
            .tick
            .saturating_sub(candidate.frame1_tick),
        grace_frames,
        target_frame_start: target_start.frame,
        target_frame_end: target_end.frame,
        target_tick_start: target_start.tick,
        target_tick_end: target_end.tick,
        representative_target_frame: target_representative.frame,
        target_grace_frames: interval_len
            .try_into()
            .unwrap_or(i64::MAX)
            .saturating_mul(2),
        candidates: smallvec::smallvec![candidate],
    });
}

pub(super) fn target_input_intervals(
    candidate: &TimeleapCandidate,
) -> Vec<Vec<TimeleapTargetInput>> {
    let mut inputs = candidate.target_inputs.to_vec();
    if inputs.is_empty() {
        inputs.push(TimeleapTargetInput {
            frame: candidate.post_frame,
            tick: candidate.post_tick,
        });
    }
    inputs.sort_by_key(|input| input.tick);
    inputs.dedup_by_key(|input| input.tick);
    let mut intervals: Vec<Vec<TimeleapTargetInput>> = Vec::new();
    for input in inputs {
        if let Some(interval) = intervals.last_mut() {
            if interval
                .last()
                .is_some_and(|previous| input.tick == previous.tick.saturating_add(1))
            {
                interval.push(input);
                continue;
            }
        }
        intervals.push(vec![input]);
    }
    intervals
}

#[cfg(test)]
pub(super) fn representative_frame(start: i64, end: i64) -> i64 {
    start.saturating_add(end.saturating_sub(start).saturating_add(1) / 2)
}

pub(super) fn representative_candidate_frame(group: &TimeleapCandidateGroup) -> i64 {
    group
        .representative_candidate()
        .map(|candidate| candidate.frame1)
        .unwrap_or(group.frame1_start)
}

pub(super) fn representative_frame2_ticks(group: &TimeleapCandidateGroup) -> i64 {
    group
        .representative_candidate()
        .map(TimeleapCandidate::frame1_to_frame2_ticks)
        .unwrap_or_default()
}
