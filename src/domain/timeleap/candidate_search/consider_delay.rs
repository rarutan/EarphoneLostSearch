use super::super::super::rng::{ObservationCache, Sfmt};
use super::super::runtime::{
    advance_one_tick_state_only_from_values, advance_one_tick_without_blink_list,
    apply_consider_delay_cancelable, apply_event_consider_delay_from_values, cached_sfmt_at,
    consider_delay_type_id, delay_elapsed_tick_bound, CourierConsumptionBoundsAccumulator,
};
#[cfg(test)]
use super::super::MAX_CONSIDER_DELAY_SCAN_PATH_STATES;
use super::super::{
    ConsiderDelayConfig, ConsiderDelayGenerator, TimeleapConfig, MAX_CANDIDATE_REPLAY_TICKS,
};
#[cfg(test)]
use super::is_merge_checkpoint;
use super::{
    advance_to_target_with_merging_with_limit, exact_future_state_key, is_cancelled,
    CachedTargetSuffix, ConsiderDelayHit, Cursor, FutureStateKey, PendingSharedMemoPath,
    SharedConsiderDelayInputResultCache, SharedConsiderDelayInputSuffixCache, SharedMemoLookup,
    SharedMemoWait, TimeleapSearchStats, TimeleapStatus,
};
#[cfg(test)]
use super::{
    CachedConsiderDelayHit, ConsiderDelayScanKey, PendingSharedScanPath,
    SharedConsiderDelayScanCache, SharedTargetSuffixCache,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

pub(super) const MAX_MEMOIZED_CONSIDER_DELAY_WINDOW_HITS: usize = 256;
const CONSIDER_DELAY_SUFFIX_CHECKPOINT_INTERVAL: usize = 16;
const EVENT_CONSUMPTION_BOUND_CHUNK_VALUES: usize = 65_536;
const EVENT_CONSUMPTION_BOUND_MAX_SINGLE_WINDOW_VALUES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::domain::timeleap) struct EventGenerationFrameBounds {
    pub(in crate::domain::timeleap) min: i64,
    pub(in crate::domain::timeleap) max: i64,
}

impl From<i64> for EventGenerationFrameBounds {
    fn from(max: i64) -> Self {
        Self {
            min: 0,
            max: max.max(0),
        }
    }
}

impl From<i32> for EventGenerationFrameBounds {
    fn from(max: i32) -> Self {
        i64::from(max).into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ConsiderDelayInputWindowKey {
    pub(super) window_start: i64,
    pub(super) state: FutureStateKey,
}

#[derive(Clone)]
pub(super) struct CachedConsiderDelayInputHit {
    input_frame: i64,
    input_sfmt_position: i64,
    input_tick_offset: i64,
    landing_frame: i64,
    landing_sfmt_position: i64,
    landing_tick_offset: i64,
    landing_status: TimeleapStatus,
}

#[derive(Clone)]
pub(super) struct CachedConsiderDelayInputSuffix {
    hits: std::sync::Arc<Vec<CachedConsiderDelayInputHit>>,
    start_index: usize,
    checkpoint_tick_offset: i64,
}

pub(in crate::domain::timeleap) fn search_consider_delay_inputs(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    status: &TimeleapStatus,
    cursor: &Cursor,
    sfmt: &Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Vec<ConsiderDelayHit>, String> {
    let mut input_window_cache = HashMap::new();
    search_consider_delay_inputs_with_window_cache(
        delay,
        delay_ticks,
        config,
        status,
        cursor,
        sfmt,
        cache,
        window_suffixes,
        &mut input_window_cache,
        cancel,
        stats,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn search_consider_delay_inputs_with_window_cache(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    status: &TimeleapStatus,
    cursor: &Cursor,
    sfmt: &Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    input_window_cache: &mut HashMap<ConsiderDelayInputWindowKey, Vec<CachedConsiderDelayInputHit>>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Vec<ConsiderDelayHit>, String> {
    let event_generation_bounds =
        max_event_generation_consumption(cache, delay, config, *cursor, cancel)?;
    search_consider_delay_inputs_with_event_bound(
        delay,
        delay_ticks,
        config,
        status,
        cursor,
        sfmt,
        cache,
        window_suffixes,
        input_window_cache,
        event_generation_bounds,
        None,
        None,
        cancel,
        stats,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn search_consider_delay_inputs_with_event_bound<B: Into<EventGenerationFrameBounds>>(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    status: &TimeleapStatus,
    cursor: &Cursor,
    sfmt: &Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    input_window_cache: &mut HashMap<ConsiderDelayInputWindowKey, Vec<CachedConsiderDelayInputHit>>,
    event_generation_bounds: B,
    shared_consider_delay_input_results: Option<&SharedConsiderDelayInputResultCache>,
    shared_consider_delay_input_suffixes: Option<&SharedConsiderDelayInputSuffixCache>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Vec<ConsiderDelayHit>, String> {
    let event_generation_bounds = event_generation_bounds.into();
    search_consider_delay_inputs_with_event_bound_internal(
        delay,
        delay_ticks,
        config,
        status,
        cursor,
        sfmt,
        cache,
        window_suffixes,
        input_window_cache,
        event_generation_bounds,
        shared_consider_delay_input_results,
        shared_consider_delay_input_suffixes,
        None,
        cancel,
        stats,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn search_consider_delay_inputs_with_event_bound_and_values<
    B: Into<EventGenerationFrameBounds>,
>(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    status: &TimeleapStatus,
    cursor: &Cursor,
    sfmt: &Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    input_window_cache: &mut HashMap<ConsiderDelayInputWindowKey, Vec<CachedConsiderDelayInputHit>>,
    event_generation_bounds: B,
    shared_consider_delay_input_results: Option<&SharedConsiderDelayInputResultCache>,
    shared_consider_delay_input_suffixes: Option<&SharedConsiderDelayInputSuffixCache>,
    values_start: i64,
    values: &[u64],
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Vec<ConsiderDelayHit>, String> {
    let event_generation_bounds = event_generation_bounds.into();
    search_consider_delay_inputs_with_event_bound_internal(
        delay,
        delay_ticks,
        config,
        status,
        cursor,
        sfmt,
        cache,
        window_suffixes,
        input_window_cache,
        event_generation_bounds,
        shared_consider_delay_input_results,
        shared_consider_delay_input_suffixes,
        Some((values_start, values)),
        cancel,
        stats,
    )
}

#[allow(clippy::too_many_arguments)]
fn search_consider_delay_inputs_with_event_bound_internal(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    status: &TimeleapStatus,
    cursor: &Cursor,
    sfmt: &Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    input_window_cache: &mut HashMap<ConsiderDelayInputWindowKey, Vec<CachedConsiderDelayInputHit>>,
    event_generation_bounds: EventGenerationFrameBounds,
    shared_consider_delay_input_results: Option<&SharedConsiderDelayInputResultCache>,
    shared_consider_delay_input_suffixes: Option<&SharedConsiderDelayInputSuffixCache>,
    cached_values: Option<(i64, &[u64])>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Vec<ConsiderDelayHit>, String> {
    let (window_frames, window_end_frame_offset, uses_event7_tick_start_candidates) =
        consider_delay_initial_window_frames(
            delay,
            delay_ticks,
            config,
            event_generation_bounds,
            status.raining,
        );
    let minimum_frame = config.start_frame.max(cursor.frame);

    if is_cancelled(cancel) {
        return Err("Timeleap候補検索がキャンセルされました。".into());
    }
    let window_start = config
        .target_frame
        .saturating_sub(window_frames)
        .max(minimum_frame);
    let window_end = config
        .target_frame
        .saturating_sub(window_end_frame_offset)
        .max(window_start);
    let cache_key = ConsiderDelayInputWindowKey {
        window_start,
        state: exact_future_state_key(*cursor, status),
    };
    let mut pending_shared_results =
        PendingSharedMemoPath::<Vec<CachedConsiderDelayInputHit>>::default();
    if let Some(shared_results) = shared_consider_delay_input_results {
        loop {
            match shared_results.lookup_or_reserve(&cache_key) {
                SharedMemoLookup::Ready(cached_hits) => {
                    stats.shared_consider_delay_input_result_hits = stats
                        .shared_consider_delay_input_result_hits
                        .saturating_add(1);
                    return Ok(restore_cached_consider_delay_input_hits(
                        &cached_hits,
                        cursor,
                    ));
                }
                SharedMemoLookup::Owner(entry) => {
                    stats.shared_consider_delay_input_result_misses = stats
                        .shared_consider_delay_input_result_misses
                        .saturating_add(1);
                    pending_shared_results.claim(entry, cursor.tick);
                    break;
                }
                SharedMemoLookup::Wait(entry) => {
                    stats.shared_consider_delay_input_result_waits = stats
                        .shared_consider_delay_input_result_waits
                        .saturating_add(1);
                    match shared_results.wait(&entry, cancel)? {
                        SharedMemoWait::Ready(cached_hits) => {
                            stats.shared_consider_delay_input_result_hits = stats
                                .shared_consider_delay_input_result_hits
                                .saturating_add(1);
                            return Ok(restore_cached_consider_delay_input_hits(
                                &cached_hits,
                                cursor,
                            ));
                        }
                        SharedMemoWait::Retry => continue,
                    }
                }
                SharedMemoLookup::Uncached => {
                    stats.shared_consider_delay_input_result_bypasses = stats
                        .shared_consider_delay_input_result_bypasses
                        .saturating_add(1);
                    break;
                }
            }
        }
    }
    // All generators share the tick-start scan. Event7 alone can also apply
    // its delay directly from cached SFMT values; fixed-symbol policies retain
    // their own SFMT-backed transition.
    let event_delay_values = uses_event7_tick_start_candidates
        .then_some(cached_values)
        .flatten();
    let hits = search_delay_inputs_by_tick_start(
        delay,
        config,
        status,
        cursor,
        sfmt,
        cache,
        window_suffixes,
        input_window_cache,
        window_start,
        window_end,
        shared_consider_delay_input_suffixes,
        event_delay_values,
        event_delay_values,
        cancel,
        stats,
    )?;
    if hits.len() <= MAX_MEMOIZED_CONSIDER_DELAY_WINDOW_HITS {
        let cached_hits = cache_consider_delay_input_hits(&hits, cursor.tick);
        pending_shared_results.complete_with(|_| cached_hits.clone());
    }
    Ok(hits)
}

#[allow(clippy::too_many_arguments)]
fn search_delay_inputs_by_tick_start(
    delay: &ConsiderDelayConfig,
    config: &TimeleapConfig,
    status: &TimeleapStatus,
    cursor: &Cursor,
    sfmt: &Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    input_window_cache: &mut HashMap<ConsiderDelayInputWindowKey, Vec<CachedConsiderDelayInputHit>>,
    window_start: i64,
    window_end: i64,
    shared_consider_delay_input_suffixes: Option<&SharedConsiderDelayInputSuffixCache>,
    cached_values: Option<(i64, &[u64])>,
    event_cached_values: Option<(i64, &[u64])>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Vec<ConsiderDelayHit>, String> {
    // All post-Timeleap generators use one A-input candidate per tick-start,
    // matching Search7_TimelineLeap1 rather than each consumed display Frame.
    // Generator-specific delay semantics remain in the delay-application call
    // below; notably, Event7's delay ticks do not advance menu fidget.
    let aligned_start = window_start;
    let mut scan_status = status.clone();
    let mut scan_cursor = *cursor;
    let mut scan_sfmt = sfmt.clone();
    let cached_suffix_count = window_suffixes.values().map(HashMap::len).sum::<usize>();
    let suffixes = window_suffixes.entry(aligned_start).or_default();
    advance_to_target_with_merging_with_limit(
        aligned_start,
        &mut scan_status,
        &mut scan_cursor,
        &mut scan_sfmt,
        Some(suffixes),
        None,
        cancel,
        stats,
        usize::MAX.saturating_sub(cached_suffix_count),
    )?;
    scan_sfmt = cached_sfmt_at(cache, scan_cursor.sfmt_position)?;

    let scan_key = ConsiderDelayInputWindowKey {
        window_start,
        state: exact_future_state_key(scan_cursor, &scan_status),
    };
    let scan_start_tick = scan_cursor.tick;
    if let Some(cached_hits) = input_window_cache.get(&scan_key) {
        stats.consider_delay_input_window_cache_hits = stats
            .consider_delay_input_window_cache_hits
            .saturating_add(1);
        return Ok(cached_hits
            .iter()
            .map(|hit| ConsiderDelayHit {
                input_cursor: Cursor {
                    frame: hit.input_frame,
                    sfmt_position: hit.input_sfmt_position,
                    tick: scan_cursor.tick.saturating_add(hit.input_tick_offset),
                },
                landing_cursor: Cursor {
                    frame: hit.landing_frame,
                    sfmt_position: hit.landing_sfmt_position,
                    tick: scan_cursor.tick.saturating_add(hit.landing_tick_offset),
                },
                landing_status: hit.landing_status.clone(),
            })
            .collect());
    }
    stats.consider_delay_input_window_cache_misses = stats
        .consider_delay_input_window_cache_misses
        .saturating_add(1);

    let mut hits = Vec::new();
    let mut pending_shared_suffixes =
        PendingSharedMemoPath::<CachedConsiderDelayInputSuffix>::default();
    let mut reused_shared_suffix = false;
    let mut ticks_scanned = 0usize;
    let mut tick_start_status = scan_status.clone();
    let mut next_status = scan_status.clone();
    let mut landing_status = scan_status.clone();
    let mut next_sfmt = scan_sfmt.clone();
    while scan_cursor.frame <= window_end && ticks_scanned < MAX_CANDIDATE_REPLAY_TICKS {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }

        if let Some(shared_suffix_cache) = shared_consider_delay_input_suffixes {
            let should_check_shared_cache = ticks_scanned == 0
                || ticks_scanned % CONSIDER_DELAY_SUFFIX_CHECKPOINT_INTERVAL == 0;
            if should_check_shared_cache {
                let key = ConsiderDelayInputWindowKey {
                    window_start,
                    state: exact_future_state_key(scan_cursor, &scan_status),
                };
                loop {
                    match shared_suffix_cache.lookup_or_reserve(&key) {
                        SharedMemoLookup::Ready(suffix) => {
                            stats.shared_consider_delay_input_suffix_hits = stats
                                .shared_consider_delay_input_suffix_hits
                                .saturating_add(1);
                            hits.extend(restore_cached_consider_delay_input_suffix(
                                &suffix,
                                &scan_cursor,
                            ));
                            reused_shared_suffix = true;
                            break;
                        }
                        SharedMemoLookup::Owner(entry) => {
                            stats.shared_consider_delay_input_suffix_misses = stats
                                .shared_consider_delay_input_suffix_misses
                                .saturating_add(1);
                            pending_shared_suffixes.claim(entry, scan_cursor.tick);
                            break;
                        }
                        SharedMemoLookup::Wait(entry) => {
                            stats.shared_consider_delay_input_suffix_waits = stats
                                .shared_consider_delay_input_suffix_waits
                                .saturating_add(1);
                            match shared_suffix_cache.wait(&entry, cancel)? {
                                SharedMemoWait::Ready(suffix) => {
                                    stats.shared_consider_delay_input_suffix_hits = stats
                                        .shared_consider_delay_input_suffix_hits
                                        .saturating_add(1);
                                    hits.extend(restore_cached_consider_delay_input_suffix(
                                        &suffix,
                                        &scan_cursor,
                                    ));
                                    reused_shared_suffix = true;
                                    break;
                                }
                                SharedMemoWait::Retry => continue,
                            }
                        }
                        SharedMemoLookup::Uncached => {
                            stats.shared_consider_delay_input_suffix_bypasses = stats
                                .shared_consider_delay_input_suffix_bypasses
                                .saturating_add(1);
                            break;
                        }
                    }
                }
                if reused_shared_suffix {
                    break;
                }
            }
        }

        let tick_start_cursor = scan_cursor;
        tick_start_status.clone_from(&scan_status);
        next_status.clone_from(&scan_status);
        let mut next_cursor = scan_cursor;
        next_sfmt.clone_from(&scan_sfmt);
        let active_models = next_status.model.remain.len();
        let advanced_from_values = cached_values.is_some_and(|(values_start, values)| {
            advance_one_tick_state_only_from_values(
                &mut next_status,
                &mut next_cursor,
                active_models,
                true,
                values_start,
                values,
            )
        });
        if !advanced_from_values {
            if cached_values.is_some() {
                next_sfmt = cached_sfmt_at(cache, scan_cursor.sfmt_position)?;
            }
            advance_one_tick_without_blink_list(&mut next_status, &mut next_cursor, &mut next_sfmt);
        }

        let input_cursor = tick_start_cursor;
        if (window_start..=window_end).contains(&input_cursor.frame) {
            if is_cancelled(cancel) {
                return Err("Timeleap候補検索がキャンセルされました。".into());
            }
            landing_status.clone_from(&tick_start_status);
            let mut landing_cursor = input_cursor;
            let reached = if let Some((values_start, values)) = event_cached_values {
                apply_event_consider_delay_from_values(
                    delay,
                    &mut landing_status,
                    &mut landing_cursor,
                    config.models,
                    config.target_frame,
                    values_start,
                    values,
                    cancel,
                )?
            } else {
                let mut landing_sfmt = scan_sfmt.clone();
                apply_consider_delay_cancelable(
                    delay,
                    &mut landing_status,
                    &mut landing_cursor,
                    &mut landing_sfmt,
                    config.models,
                    config.target_frame,
                    cancel,
                )?
            };
            if reached {
                hits.push(ConsiderDelayHit {
                    input_cursor,
                    landing_cursor,
                    landing_status: landing_status.clone(),
                });
            }
        }

        std::mem::swap(&mut scan_status, &mut next_status);
        scan_cursor = next_cursor;
        std::mem::swap(&mut scan_sfmt, &mut next_sfmt);
        stats.candidate_ticks_simulated = stats.candidate_ticks_simulated.saturating_add(1);
        ticks_scanned += 1;
    }
    if ticks_scanned >= MAX_CANDIDATE_REPLAY_TICKS {
        return Err("Consider DelayのA入力候補検索が上限を超えました。".into());
    }
    if hits.len() <= MAX_MEMOIZED_CONSIDER_DELAY_WINDOW_HITS {
        let cached_hits = cache_consider_delay_input_hits(&hits, scan_start_tick);
        input_window_cache.insert(scan_key, cached_hits.clone());
        if shared_consider_delay_input_suffixes.is_some() {
            let shared_hits = std::sync::Arc::new(cached_hits);
            pending_shared_suffixes.complete_with(|checkpoint_tick| {
                let checkpoint_tick_offset = checkpoint_tick.saturating_sub(scan_start_tick);
                let start_index = shared_hits
                    .partition_point(|hit| hit.input_tick_offset < checkpoint_tick_offset);
                CachedConsiderDelayInputSuffix {
                    hits: shared_hits.clone(),
                    start_index,
                    checkpoint_tick_offset,
                }
            });
        }
    }
    Ok(hits)
}

pub(super) fn restore_cached_consider_delay_input_hits(
    cached_hits: &[CachedConsiderDelayInputHit],
    cursor: &Cursor,
) -> Vec<ConsiderDelayHit> {
    cached_hits
        .iter()
        .map(|hit| ConsiderDelayHit {
            input_cursor: Cursor {
                frame: hit.input_frame,
                sfmt_position: hit.input_sfmt_position,
                tick: cursor.tick.saturating_add(hit.input_tick_offset),
            },
            landing_cursor: Cursor {
                frame: hit.landing_frame,
                sfmt_position: hit.landing_sfmt_position,
                tick: cursor.tick.saturating_add(hit.landing_tick_offset),
            },
            landing_status: hit.landing_status.clone(),
        })
        .collect()
}

pub(super) fn cache_consider_delay_input_hits(
    hits: &[ConsiderDelayHit],
    base_tick: i64,
) -> Vec<CachedConsiderDelayInputHit> {
    hits.iter()
        .map(|hit| CachedConsiderDelayInputHit {
            input_frame: hit.input_cursor.frame,
            input_sfmt_position: hit.input_cursor.sfmt_position,
            input_tick_offset: hit.input_cursor.tick.saturating_sub(base_tick),
            landing_frame: hit.landing_cursor.frame,
            landing_sfmt_position: hit.landing_cursor.sfmt_position,
            landing_tick_offset: hit.landing_cursor.tick.saturating_sub(base_tick),
            landing_status: hit.landing_status.clone(),
        })
        .collect()
}

fn restore_cached_consider_delay_input_suffix(
    suffix: &CachedConsiderDelayInputSuffix,
    cursor: &Cursor,
) -> Vec<ConsiderDelayHit> {
    suffix.hits[suffix.start_index..]
        .iter()
        .map(|hit| ConsiderDelayHit {
            input_cursor: Cursor {
                frame: hit.input_frame,
                sfmt_position: hit.input_sfmt_position,
                tick: cursor.tick.saturating_add(
                    hit.input_tick_offset
                        .saturating_sub(suffix.checkpoint_tick_offset),
                ),
            },
            landing_cursor: Cursor {
                frame: hit.landing_frame,
                sfmt_position: hit.landing_sfmt_position,
                tick: cursor.tick.saturating_add(
                    hit.landing_tick_offset
                        .saturating_sub(suffix.checkpoint_tick_offset),
                ),
            },
            landing_status: hit.landing_status.clone(),
        })
        .collect()
}

#[cfg(test)]
pub(super) fn search_consider_delay_input(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    _event_start_tick: i64,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    window_results: Option<&mut HashMap<ConsiderDelayScanKey, Option<CachedConsiderDelayHit>>>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
) -> Result<Option<(Cursor, TimeleapStatus)>, String> {
    search_consider_delay_input_with_cache_limits(
        delay,
        delay_ticks,
        config,
        _event_start_tick,
        status,
        cursor,
        sfmt,
        cache,
        window_suffixes,
        None,
        window_results,
        None,
        cancel,
        stats,
        usize::MAX,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn search_consider_delay_input_with_cache_limits(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    _event_start_tick: i64,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    cache: &ObservationCache,
    window_suffixes: &mut HashMap<i64, HashMap<FutureStateKey, CachedTargetSuffix>>,
    shared_window_suffixes: Option<&SharedTargetSuffixCache>,
    mut window_results: Option<&mut HashMap<ConsiderDelayScanKey, Option<CachedConsiderDelayHit>>>,
    shared_window_results: Option<&SharedConsiderDelayScanCache>,
    cancel: Option<&AtomicBool>,
    stats: &mut TimeleapSearchStats,
    suffix_cache_limit: usize,
) -> Result<Option<(Cursor, TimeleapStatus)>, String> {
    let initial_status = status.clone();
    let initial_cursor = *cursor;
    let initial_sfmt = sfmt.clone();
    let event_generation_bounds =
        max_event_generation_consumption(cache, delay, config, initial_cursor, cancel)?;
    let (window_frames, window_end_frame_offset, _) = consider_delay_initial_window_frames(
        delay,
        delay_ticks,
        config,
        event_generation_bounds,
        status.raining,
    );
    let minimum_frame = config.start_frame.max(initial_cursor.frame);

    if is_cancelled(cancel) {
        return Err("Timeleap候補検索がキャンセルされました。".into());
    }
    let window_start = config
        .target_frame
        .saturating_sub(window_frames)
        .max(minimum_frame);
    let window_end = config
        .target_frame
        .saturating_sub(window_end_frame_offset)
        .max(window_start);
    let mut scan_status = initial_status.clone();
    let mut scan_cursor = initial_cursor;
    let mut scan_sfmt = initial_sfmt.clone();
    let cached_suffix_count = window_suffixes.values().map(HashMap::len).sum::<usize>();
    let suffixes = window_suffixes.entry(window_start).or_default();
    advance_to_target_with_merging_with_limit(
        window_start,
        &mut scan_status,
        &mut scan_cursor,
        &mut scan_sfmt,
        Some(suffixes),
        shared_window_suffixes,
        cancel,
        stats,
        suffix_cache_limit.saturating_sub(cached_suffix_count),
    )?;
    // A cached state jump updates the deterministic state but not this local
    // SFMT cursor. Restore it at the resulting absolute cache position.
    scan_sfmt = cached_sfmt_at(cache, scan_cursor.sfmt_position)?;

    let mut ticks_scanned = 0usize;
    let mut scan_path = Vec::new();
    let mut shared_scan_path = PendingSharedScanPath::default();
    let mut last_checkpoint_key = None;
    let mut reused_no_hit_suffix = false;
    while scan_cursor.frame <= window_end && ticks_scanned < MAX_CANDIDATE_REPLAY_TICKS {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        let is_scan_checkpoint =
            ticks_scanned == 0 || is_merge_checkpoint(scan_cursor.sfmt_position);
        if is_scan_checkpoint {
            let key = ConsiderDelayScanKey {
                window_start,
                state: exact_future_state_key(scan_cursor, &scan_status),
            };
            if last_checkpoint_key.as_ref() != Some(&key) {
                let mut cached = window_results
                    .as_deref()
                    .and_then(|results| results.get(&key).cloned());
                if cached.is_none() {
                    if let Some(shared_results) = shared_window_results {
                        loop {
                            match shared_results.lookup_or_reserve(&key) {
                                SharedMemoLookup::Ready(result) => {
                                    cached = Some(result);
                                    break;
                                }
                                SharedMemoLookup::Owner(entry) => {
                                    shared_scan_path.claim(entry, scan_cursor.tick);
                                    break;
                                }
                                SharedMemoLookup::Wait(entry) => {
                                    stats.consider_delay_scan_waits =
                                        stats.consider_delay_scan_waits.saturating_add(1);
                                    match shared_results.wait(&entry, cancel)? {
                                        SharedMemoWait::Ready(result) => {
                                            cached = Some(result);
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
                if let Some(cached) = cached {
                    stats.consider_delay_scan_reuse_hits =
                        stats.consider_delay_scan_reuse_hits.saturating_add(1);
                    if window_results.is_some()
                        && scan_path.len() < MAX_CONSIDER_DELAY_SCAN_PATH_STATES
                    {
                        scan_path.push((key.clone(), scan_cursor.tick));
                    }
                    if let Some(hit) = cached {
                        let input_cursor = Cursor {
                            frame: hit.input_frame,
                            sfmt_position: hit.input_sfmt_position,
                            tick: scan_cursor.tick.saturating_add(hit.input_tick_offset),
                        };
                        let landing_cursor = Cursor {
                            frame: hit.landing_frame,
                            sfmt_position: hit.landing_sfmt_position,
                            tick: scan_cursor.tick.saturating_add(hit.landing_tick_offset),
                        };
                        remember_consider_delay_hits(
                            window_results.as_deref_mut(),
                            &scan_path,
                            input_cursor,
                            landing_cursor,
                            &hit.landing_status,
                        );
                        shared_scan_path.complete_with(|tick| {
                            Some(CachedConsiderDelayHit {
                                input_frame: input_cursor.frame,
                                input_sfmt_position: input_cursor.sfmt_position,
                                input_tick_offset: input_cursor.tick.saturating_sub(tick),
                                landing_frame: landing_cursor.frame,
                                landing_sfmt_position: landing_cursor.sfmt_position,
                                landing_tick_offset: landing_cursor.tick.saturating_sub(tick),
                                landing_status: hit.landing_status.clone(),
                            })
                        });
                        *status = hit.landing_status.clone();
                        *cursor = input_cursor;
                        *sfmt = cached_sfmt_at(cache, input_cursor.sfmt_position)?;
                        return Ok(Some((landing_cursor, hit.landing_status)));
                    }
                    remember_consider_delay_misses(window_results.as_deref_mut(), &scan_path);
                    shared_scan_path.complete_with(|_| None);
                    reused_no_hit_suffix = true;
                    break;
                }
                if window_results.is_some() && scan_path.len() < MAX_CONSIDER_DELAY_SCAN_PATH_STATES
                {
                    scan_path.push((key.clone(), scan_cursor.tick));
                }
                last_checkpoint_key = Some(key);
            }
        }
        let mut input_status = scan_status.clone();
        let mut input_cursor = scan_cursor;
        let mut input_sfmt = scan_sfmt.clone();
        if apply_consider_delay_cancelable(
            delay,
            &mut input_status,
            &mut input_cursor,
            &mut input_sfmt,
            config.models,
            config.target_frame,
            cancel,
        )? {
            remember_consider_delay_hits(
                window_results.as_deref_mut(),
                &scan_path,
                scan_cursor,
                input_cursor,
                &input_status,
            );
            shared_scan_path.complete_with(|tick| {
                Some(CachedConsiderDelayHit {
                    input_frame: scan_cursor.frame,
                    input_sfmt_position: scan_cursor.sfmt_position,
                    input_tick_offset: scan_cursor.tick.saturating_sub(tick),
                    landing_frame: input_cursor.frame,
                    landing_sfmt_position: input_cursor.sfmt_position,
                    landing_tick_offset: input_cursor.tick.saturating_sub(tick),
                    landing_status: input_status.clone(),
                })
            });
            *status = input_status.clone();
            *cursor = scan_cursor;
            *sfmt = scan_sfmt;
            return Ok(Some((input_cursor, input_status)));
        }
        if scan_cursor.frame == config.target_frame {
            break;
        }
        advance_one_tick_without_blink_list(&mut scan_status, &mut scan_cursor, &mut scan_sfmt);
        stats.candidate_ticks_simulated = stats.candidate_ticks_simulated.saturating_add(1);
        ticks_scanned += 1;
    }

    if ticks_scanned >= MAX_CANDIDATE_REPLAY_TICKS {
        return Ok(None);
    }
    if !reused_no_hit_suffix {
        remember_consider_delay_misses(window_results.as_deref_mut(), &scan_path);
    }
    shared_scan_path.complete_with(|_| None);
    Ok(None)
}

pub(in crate::domain::timeleap) fn consider_delay_initial_window_frames<
    B: Into<EventGenerationFrameBounds>,
>(
    delay: &ConsiderDelayConfig,
    delay_ticks: i32,
    config: &TimeleapConfig,
    event_generation_bounds: B,
    raining: bool,
) -> (i64, i64, bool) {
    let event_generation_bounds = event_generation_bounds.into();
    let maximum_models = match delay.generator {
        ConsiderDelayGenerator::FixedSymbol(_) => config.models.max(4),
        ConsiderDelayGenerator::Event => config.models,
    };
    let delay_type_id = consider_delay_type_id(delay);
    let direct_extra = match delay.generator {
        ConsiderDelayGenerator::FixedSymbol(_) => match delay_type_id {
            4 => 2,
            6 => 3,
            1 | 2 | 3 | 9 | 13 | 23 => 1,
            10..=17 | 21 | 24..=27 => 1,
            22 => 3,
            _ => 0,
        },
        ConsiderDelayGenerator::Event => 0,
    };
    // Event7 Delay advances with menu-fidget disabled, so its tick bound is one
    // output per model plus one only when this Timeline is rainy. Fixed-symbol
    // routes retain the broader bound for their distinct delay policies.
    let outputs_per_tick = match delay.generator {
        ConsiderDelayGenerator::Event => maximum_models.saturating_add(usize::from(raining)),
        ConsiderDelayGenerator::FixedSymbol(_) => maximum_models.saturating_add(4),
    };
    let elapsed_ticks = delay_elapsed_tick_bound(delay, delay_ticks);
    let window_frames = i64::from(elapsed_ticks.max(0))
        .saturating_mul(outputs_per_tick as i64)
        .saturating_add(direct_extra)
        .saturating_add(event_generation_bounds.max.max(0));
    // Event7 delay ticks can consume zero outputs when every model is still
    // counting down. Its safe lower bound is therefore only the minimum
    // completed generation length. Fixed-symbol routes retain their old scan
    // through TargetFrame.
    let minimum_elapsed_frames = if matches!(delay.generator, ConsiderDelayGenerator::Event) {
        event_generation_bounds.min.max(0)
    } else {
        0
    };
    // Event7 post-Timeleap Consider Delay follows Search7_TimelineLeap1: one
    // candidate at each tick-start, regardless of whether No Dex triggers Generate.
    let uses_event7_tick_start_candidates =
        matches!(delay.generator, ConsiderDelayGenerator::Event);
    (
        window_frames,
        minimum_elapsed_frames,
        uses_event7_tick_start_candidates,
    )
}

/// Event7の生成消費回数を、候補TimelineがTargetまでに通り得るSFMT位置から測る。
/// SFMT位置とFrame位置は遷移中に同じ出力数だけ進むため、開始位置からTargetまでの
/// 絶対SFMT区間は全候補Timelineを覆う上限になる。実際の候補開始位置はこの区間の部分集合。
pub(in crate::domain::timeleap) fn max_event_generation_consumption(
    cache: &ObservationCache,
    delay: &ConsiderDelayConfig,
    config: &TimeleapConfig,
    cursor: Cursor,
    cancel: Option<&AtomicBool>,
) -> Result<EventGenerationFrameBounds, String> {
    if !matches!(delay.generator, ConsiderDelayGenerator::Event)
        || !delay.courier.generates_during_event_delay()
    {
        return Ok(EventGenerationFrameBounds::default());
    }

    let start = cursor.sfmt_position.max(0);
    let remaining_frames = config.target_frame.saturating_sub(cursor.frame).max(0);
    let end_exclusive = start.saturating_add(remaining_frames);
    if start >= end_exclusive {
        return Ok(EventGenerationFrameBounds::default());
    }
    if end_exclusive.saturating_sub(1) > cache.available_end() {
        return Err("Consider DelayのEvent7消費数を測るSFMT範囲がありません。".into());
    }

    let scan_len = usize::try_from(end_exclusive.saturating_sub(start)).unwrap_or(usize::MAX);
    let mut accumulator = CourierConsumptionBoundsAccumulator::new(&delay.courier, scan_len);
    if scan_len <= EVENT_CONSUMPTION_BOUND_MAX_SINGLE_WINDOW_VALUES {
        let values = cache
            .read_values(start, scan_len)
            .map_err(|error| error.to_string())?;
        for (index, value) in values.iter().enumerate().rev() {
            if index % 32 == 0 && is_cancelled(cancel) {
                return Err("Timeleap候補検索がキャンセルされました。".into());
            }
            accumulator.push_reverse(index, *value);
        }
    } else {
        let mut block_end = scan_len;
        while block_end > 0 {
            if is_cancelled(cancel) {
                return Err("Timeleap候補検索がキャンセルされました。".into());
            }
            let block_start = block_end.saturating_sub(EVENT_CONSUMPTION_BOUND_CHUNK_VALUES);
            let block_len = block_end - block_start;
            let values = cache
                .read_values(start.saturating_add(block_start as i64), block_len)
                .map_err(|error| error.to_string())?;
            for (offset, value) in values.iter().enumerate().rev() {
                let index = block_start.saturating_add(offset);
                if index % 32 == 0 && is_cancelled(cancel) {
                    return Err("Timeleap候補検索がキャンセルされました。".into());
                }
                accumulator.push_reverse(index, *value);
            }
            block_end = block_start;
        }
    }
    let bounds = accumulator.finish();
    Ok(EventGenerationFrameBounds {
        min: bounds
            .min
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX))
            .unwrap_or(0),
        max: i64::try_from(bounds.max).unwrap_or(i64::MAX),
    })
}

#[cfg(test)]
pub(super) fn remember_consider_delay_hits(
    results: Option<&mut HashMap<ConsiderDelayScanKey, Option<CachedConsiderDelayHit>>>,
    path: &[(ConsiderDelayScanKey, i64)],
    input: Cursor,
    landing: Cursor,
    landing_status: &TimeleapStatus,
) {
    let Some(results) = results else {
        return;
    };
    for (key, tick) in path {
        results.entry(key.clone()).or_insert_with(|| {
            Some(CachedConsiderDelayHit {
                input_frame: input.frame,
                input_sfmt_position: input.sfmt_position,
                input_tick_offset: input.tick.saturating_sub(*tick),
                landing_frame: landing.frame,
                landing_sfmt_position: landing.sfmt_position,
                landing_tick_offset: landing.tick.saturating_sub(*tick),
                landing_status: landing_status.clone(),
            })
        });
    }
}

#[cfg(test)]
pub(super) fn remember_consider_delay_misses(
    results: Option<&mut HashMap<ConsiderDelayScanKey, Option<CachedConsiderDelayHit>>>,
    path: &[(ConsiderDelayScanKey, i64)],
) {
    let Some(results) = results else {
        return;
    };
    for (key, _) in path {
        results.entry(key.clone()).or_insert(None);
    }
}
