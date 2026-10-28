#[cfg(test)]
use super::super::rng::ModelConsumption;
use super::super::rng::{ModelStatus, ObservationCache, Sfmt};
use super::candidate_search::is_cancelled;
use super::candidate_search::{Cursor, TimeleapCandidate, TimeleapContinuation, TimeleapStatus};
#[cfg(test)]
use super::MAX_NORMAL_TIMELINE_TICKS;
#[cfg(test)]
use super::{record_model_rng_event, record_model_rng_events};
use super::{
    ConsiderDelayConfig, ConsiderDelayGenerator, CourierConfig, CourierGenerationProfile,
    TimeleapConfig, TimeleapMode, MAX_CANDIDATE_REPLAY_TICKS,
};
use std::sync::atomic::AtomicBool;

const MAX_NO_BLINK_SCAN_OUTPUTS: usize = 4_096;
#[cfg(test)]
const COURIER_CONSUMPTION_CANCEL_INTERVAL: usize = 32;

#[cfg(test)]
mod tests;

pub(super) fn validate_config(config: &TimeleapConfig) -> Result<(), String> {
    if config.start_frame < 0
        || config.target_frame < config.start_frame
        || config.target_frame > crate::domain::rng::MAX_SFMT_FRAME
        || config.models == 0
        || config.min_frame1_elapsed_ticks < 0
        || config.max_frame1_elapsed_ticks < config.min_frame1_elapsed_ticks
        || config.handoff_range_frames < 0
        || config.handoff_range_frames > 100_000
    {
        return Err("Timeleapの入力範囲を確認してください。".into());
    }
    let fixed_count = config
        .courier
        .fixed_ivs
        .iter()
        .filter(|value| **value >= 0)
        .count();
    if config.courier.iv_count as usize > 6usize.saturating_sub(fixed_count) {
        return Err("配達員の確定IV数とV個体数の組み合わせを確認してください。".into());
    }
    Ok(())
}

pub(super) fn cached_sfmt_at(cache: &ObservationCache, position: i64) -> Result<Sfmt, String> {
    const SFMT_OUTPUT_BLOCK_VALUES: i64 = 312;
    let block_end = position
        .div_euclid(SFMT_OUTPUT_BLOCK_VALUES)
        .saturating_add(1)
        .saturating_mul(SFMT_OUTPUT_BLOCK_VALUES)
        .saturating_sub(1);
    if position < 0 || block_end > cache.available_end() {
        return Err("Timeleapに必要なSFMTキャッシュ範囲がありません。".into());
    }
    cache
        .sfmt_at_checked(position)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Timeleapに必要なSFMTキャッシュ範囲がありません。".into())
}

#[cfg(test)]
pub(super) fn normal_target_ticks(
    config: &TimeleapConfig,
    initial_sfmt: &Sfmt,
) -> Result<i64, String> {
    normal_target_ticks_cancelable(config, initial_sfmt, None)
}

#[cfg(test)]
pub(super) fn normal_target_ticks_cancelable(
    config: &TimeleapConfig,
    initial_sfmt: &Sfmt,
    cancel: Option<&AtomicBool>,
) -> Result<i64, String> {
    let mut sfmt = initial_sfmt.clone();
    let mut status = TimeleapStatus::new(config.models);
    status.is_boy = config.is_boy;
    let mut frame = config.start_frame;
    for tick in 0..MAX_NORMAL_TIMELINE_TICKS {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        if frame == config.target_frame {
            return Ok(tick as i64);
        }
        let used = status.model.next_state(&mut sfmt).0 as i64;
        frame = frame.saturating_add(used);
        if frame == config.target_frame {
            return Ok((tick + 1) as i64);
        }
        // FuncUtil::CalcFrame reports the elapsed ticks at the last visited
        // Frame that did not pass the target. An overshooting transition is
        // not included in that value.
        if frame > config.target_frame {
            return Ok(tick as i64);
        }
    }
    Err("Targetまでの通常Timelineが検索上限を超えました。".into())
}

pub(super) fn advance_one_tick(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
) -> Vec<u8> {
    advance_one_tick_with_active_models(status, cursor, sfmt, status.model.remain.len()).0
}

pub(super) fn advance_one_tick_with_active_models(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
) -> (Vec<u8>, bool) {
    advance_one_tick_with_fidget_policy::<true, false>(status, cursor, sfmt, active_models, true)
}

#[cfg(test)]
pub(super) fn advance_one_tick_with_active_models_traced(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
) -> (Vec<u8>, bool) {
    advance_one_tick_with_fidget_policy::<true, true>(status, cursor, sfmt, active_models, true)
}

/// Advance one tick for search code that needs only the resulting state. This
/// uses the same RNG/state transition while avoiding a blink-index allocation.
pub(super) fn advance_one_tick_without_blink_list(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
) {
    let active_models = status.model.remain.len();
    advance_one_tick_state_only_with_trace::<false>(status, cursor, sfmt, active_models, true);
}

#[inline]
fn advance_one_tick_state_only_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
    advance_fidget: bool,
) {
    let mut used = 0i64;
    if advance_fidget && status.fidget_cd > 0 {
        status.fidget_cd -= 1;
        if status.fidget_cd == 0 {
            let continuation = TimeleapContinuation::from(&*status);
            status.fidget_cd = fidget_delay_values(&continuation, sfmt, &mut cursor.sfmt_position);
            used += 2;
        }
    }

    #[cfg(test)]
    let model_used = if TRACE_RNG {
        let tick = cursor.tick;
        let mut next = || {
            let position = cursor.sfmt_position;
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            (position, sfmt.next_u64())
        };
        status
            .model
            .next_state_prefix_with_observer_without_blink_list(
                active_models,
                &mut next,
                |draw, remain_after| record_model_rng_event(tick, draw, remain_after),
            )
    } else {
        let mut next = || {
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            sfmt.next_u64()
        };
        status
            .model
            .next_state_prefix_without_blink_list(active_models, &mut next)
    };
    #[cfg(not(test))]
    let model_used = {
        let _ = TRACE_RNG;
        let mut next = || {
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            sfmt.next_u64()
        };
        status
            .model
            .next_state_prefix_without_blink_list(active_models, &mut next)
    };

    used += model_used as i64;
    if status.raining {
        status.phase = (status.phase + 1) % 2;
        sfmt.next_u64();
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        used += 1;
    }
    cursor.frame = cursor.frame.saturating_add(used);
    cursor.tick = cursor.tick.saturating_add(1);
}

/// Advance from the shared SFMT output window instead of regenerating the same
/// deterministic output suffix for each candidate. Returns false when the
/// window does not cover the whole worst-case tick; callers can safely fall
/// back to the regular SFMT transition before mutating any state.
pub(super) fn advance_one_tick_state_only_from_values(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    active_models: usize,
    advance_fidget: bool,
    values_start: i64,
    values: &[u64],
) -> bool {
    advance_one_tick_state_only_from_values_with_trace::<false>(
        status,
        cursor,
        active_models,
        advance_fidget,
        values_start,
        values,
    )
}

#[inline]
fn advance_one_tick_state_only_from_values_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    active_models: usize,
    advance_fidget: bool,
    values_start: i64,
    values: &[u64],
) -> bool {
    let Some(offset) = cursor.sfmt_position.checked_sub(values_start) else {
        return false;
    };
    let Ok(offset) = usize::try_from(offset) else {
        return false;
    };
    let max_outputs = active_models.saturating_add(3);
    if offset.saturating_add(max_outputs) > values.len() {
        return false;
    }

    let mut used = 0i64;
    let mut value_index = offset;
    if advance_fidget && status.fidget_cd > 0 {
        status.fidget_cd -= 1;
        if status.fidget_cd == 0 {
            let continuation = TimeleapContinuation::from(&*status);
            let mut next = || {
                let value = values[value_index];
                value_index += 1;
                cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
                value
            };
            status.fidget_cd = fidget_delay_values_with(&continuation, &mut next);
            used += 2;
        }
    }

    #[cfg(test)]
    let model_used = if TRACE_RNG {
        let tick = cursor.tick;
        let mut next = || {
            let position = cursor.sfmt_position;
            let value = values[value_index];
            value_index += 1;
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            (position, value)
        };
        status
            .model
            .next_state_prefix_with_observer_without_blink_list(
                active_models,
                &mut next,
                |draw, remain_after| record_model_rng_event(tick, draw, remain_after),
            )
    } else {
        let mut next = || {
            let value = values[value_index];
            value_index += 1;
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            value
        };
        status
            .model
            .next_state_prefix_without_blink_list(active_models, &mut next)
    };
    #[cfg(not(test))]
    let model_used = {
        let _ = TRACE_RNG;
        let mut next = || {
            let value = values[value_index];
            value_index += 1;
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            value
        };
        status
            .model
            .next_state_prefix_without_blink_list(active_models, &mut next)
    };

    used += model_used as i64;
    if status.raining {
        status.phase = (status.phase + 1) % 2;
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        used += 1;
    }
    cursor.frame = cursor.frame.saturating_add(used);
    cursor.tick = cursor.tick.saturating_add(1);
    true
}

fn advance_one_tick_with_fidget_policy<const COLLECT_BLINKS: bool, const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
    advance_fidget: bool,
) -> (Vec<u8>, bool) {
    if advance_safe_frame_tick_with_trace::<TRACE_RNG>(
        status,
        cursor,
        sfmt,
        active_models,
        advance_fidget,
    ) {
        return (Vec::new(), true);
    }

    let mut continuation = TimeleapContinuation::from(&*status);
    if !advance_fidget {
        // Event7's time_elapse7 advances model states but does not run the
        // normal menu-close fidget countdown or consume its two RNG values.
        continuation.fidget_cd = -1;
    }
    let (used, blinks) =
        advance_timeleap_runtime_tick_with_tick_and_models_policy::<COLLECT_BLINKS, TRACE_RNG>(
            &mut status.model,
            &mut continuation,
            &mut cursor.sfmt_position,
            sfmt,
            active_models,
            cursor.tick,
        );
    status.phase = continuation.phase;
    if advance_fidget {
        status.fidget_cd = continuation.fidget_cd;
    }
    status.is_boy = continuation.is_boy;
    status.raining = continuation.raining;
    cursor.frame = cursor.frame.saturating_add(used);
    cursor.tick = cursor.tick.saturating_add(1);
    (blinks, false)
}

#[cfg(test)]
pub(super) fn advance_one_tick_reference(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
) -> Vec<u8> {
    advance_one_tick_reference_with_fidget_policy(status, cursor, sfmt, true)
}

#[cfg(test)]
fn advance_one_tick_reference_with_fidget_policy(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    advance_fidget: bool,
) -> Vec<u8> {
    let mut continuation = TimeleapContinuation::from(&*status);
    let mut used = 0i64;
    if advance_fidget && continuation.fidget_cd > 0 {
        continuation.fidget_cd -= 1;
        if continuation.fidget_cd == 0 {
            continuation.fidget_cd =
                fidget_delay_values(&continuation, sfmt, &mut cursor.sfmt_position);
            used += 2;
        }
    }
    let (model_used, consumed) = status.model.next_state_with_positions(|| {
        let position = cursor.sfmt_position;
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        (position, sfmt.next_u64())
    });
    record_model_rng_events(cursor.tick, &consumed, &status.model.remain);
    used += model_used as i64;
    let blinks = consumed
        .into_iter()
        .filter_map(|event| event.blink.then_some(event.model as u8))
        .collect();
    if continuation.raining {
        continuation.phase = (continuation.phase + 1) % 2;
        sfmt.next_u64();
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        used += 1;
    }
    status.phase = continuation.phase;
    if advance_fidget {
        status.fidget_cd = continuation.fidget_cd;
    }
    status.is_boy = continuation.is_boy;
    status.raining = continuation.raining;
    cursor.frame = cursor.frame.saturating_add(used);
    cursor.tick = cursor.tick.saturating_add(1);
    blinks
}

/// 全有効モデルがremain=0/1なら、状態分岐と消費記録Vecを作らず1 tick進める。
/// 乱数値の読取順と、瞬き条件が成立した時点のremain=-5への遷移は通常経路と同じ。
fn advance_safe_frame_tick_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
    advance_fidget: bool,
) -> bool {
    if status.model.remain.len() < active_models
        || (active_models > 0 && !matches!(status.model.remain[0], 0 | 1))
        || status.model.remain[..active_models]
            .iter()
            .any(|remain| !matches!(*remain, 0 | 1))
        || (advance_fidget && status.fidget_cd == 1)
    {
        return false;
    }

    if advance_fidget && status.fidget_cd > 1 {
        status.fidget_cd -= 1;
    }

    for (model, remain) in status.model.remain[..active_models].iter_mut().enumerate() {
        #[cfg(not(test))]
        let _ = model;
        #[cfg(test)]
        let position = cursor.sfmt_position;
        let value = sfmt.next_u64();
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        cursor.frame = cursor.frame.saturating_add(1);
        if value & 0x7f == 0 {
            *remain = -5;
        }
        #[cfg(test)]
        if TRACE_RNG {
            record_model_rng_event(
                cursor.tick,
                ModelConsumption {
                    model,
                    position,
                    value,
                    blink: false,
                },
                *remain,
            );
        }
    }

    if status.raining {
        status.phase = (status.phase + 1) % 2;
        sfmt.next_u64();
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        cursor.frame = cursor.frame.saturating_add(1);
    }
    #[cfg(not(test))]
    let _ = TRACE_RNG;
    cursor.tick = cursor.tick.saturating_add(1);

    true
}

/// 1 tick分だけTimeleap状態を進める共通遷移。
/// 通常Timelineへ引き継がれたfidget等も候補予測と同じSFMT消費順で処理する。
pub(crate) fn advance_timeleap_runtime_tick(
    model: &mut ModelStatus,
    continuation: &mut TimeleapContinuation,
    stream_cursor: &mut i64,
    sfmt: &mut Sfmt,
) -> (i64, Vec<u8>) {
    advance_timeleap_runtime_tick_with_tick(model, continuation, stream_cursor, sfmt, -1)
}

pub(super) fn advance_timeleap_runtime_tick_with_tick(
    model: &mut ModelStatus,
    continuation: &mut TimeleapContinuation,
    stream_cursor: &mut i64,
    sfmt: &mut Sfmt,
    tick: i64,
) -> (i64, Vec<u8>) {
    let active_models = model.remain.len();
    advance_timeleap_runtime_tick_with_tick_and_models(
        model,
        continuation,
        stream_cursor,
        sfmt,
        active_models,
        tick,
    )
}

pub(super) fn advance_timeleap_runtime_tick_with_tick_and_models(
    model: &mut ModelStatus,
    continuation: &mut TimeleapContinuation,
    stream_cursor: &mut i64,
    sfmt: &mut Sfmt,
    active_models: usize,
    tick: i64,
) -> (i64, Vec<u8>) {
    advance_timeleap_runtime_tick_with_tick_and_models_policy::<true, false>(
        model,
        continuation,
        stream_cursor,
        sfmt,
        active_models,
        tick,
    )
}

fn advance_timeleap_runtime_tick_with_tick_and_models_policy<
    const COLLECT_BLINKS: bool,
    const TRACE_RNG: bool,
>(
    model: &mut ModelStatus,
    continuation: &mut TimeleapContinuation,
    stream_cursor: &mut i64,
    sfmt: &mut Sfmt,
    active_models: usize,
    tick: i64,
) -> (i64, Vec<u8>) {
    #[cfg(not(test))]
    let _ = tick;
    let mut used = 0i64;
    if continuation.fidget_cd > 0 {
        continuation.fidget_cd -= 1;
        if continuation.fidget_cd == 0 {
            continuation.fidget_cd = fidget_delay_values(continuation, sfmt, stream_cursor);
            used += 2;
        }
    }
    let mut next = || {
        let position = *stream_cursor;
        *stream_cursor = stream_cursor.saturating_add(1);
        (position, sfmt.next_u64())
    };
    #[cfg(test)]
    let (model_used, blinks) = if TRACE_RNG {
        if COLLECT_BLINKS {
            model.next_state_prefix_with_observer(active_models, &mut next, |draw, remain_after| {
                record_model_rng_event(tick, draw, remain_after)
            })
        } else {
            (
                model.next_state_prefix_with_observer_without_blink_list(
                    active_models,
                    &mut next,
                    |draw, remain_after| record_model_rng_event(tick, draw, remain_after),
                ),
                Vec::new(),
            )
        }
    } else if COLLECT_BLINKS {
        model.next_state_prefix_with_observer(active_models, &mut next, |draw, remain_after| {
            let _ = (draw, remain_after);
        })
    } else {
        (
            model.next_state_prefix_with_observer_without_blink_list(
                active_models,
                &mut next,
                |draw, remain_after| record_model_rng_event(tick, draw, remain_after),
            ),
            Vec::new(),
        )
    };
    #[cfg(not(test))]
    let (model_used, blinks) = if COLLECT_BLINKS {
        let _ = TRACE_RNG;
        model.next_state_prefix_with_observer(active_models, &mut next, |_, _| {})
    } else {
        (
            model.next_state_prefix_with_observer_without_blink_list(
                active_models,
                &mut next,
                |_, _| {},
            ),
            Vec::new(),
        )
    };
    used += model_used as i64;
    if continuation.raining {
        continuation.phase = (continuation.phase + 1) % 2;
        sfmt.next_u64();
        *stream_cursor = stream_cursor.saturating_add(1);
        used += 1;
    }
    (used, blinks)
}

pub(super) fn fidget_delay_values(
    continuation: &TimeleapContinuation,
    sfmt: &mut Sfmt,
    stream_cursor: &mut i64,
) -> i32 {
    let mut next = || sfmt.next_u64();
    let value = fidget_delay_values_with(continuation, &mut next);
    *stream_cursor = stream_cursor.saturating_add(2);
    value
}

fn fidget_delay_values_with(
    continuation: &TimeleapContinuation,
    next: &mut impl FnMut() -> u64,
) -> i32 {
    let delay = (next() % 90) as i32;
    let movement = if continuation.is_boy {
        if next() & 1 == 0 {
            407
        } else {
            412
        }
    } else if next() & 1 == 0 {
        402
    } else {
        407
    };
    delay + movement
}

/// 候補のイベント後状態から、将来のShiftに対応する絶対SFMT位置を有限個だけ生成する。
#[allow(dead_code)]
pub(crate) fn predict_shift_positions(
    cache: &ObservationCache,
    candidate: &TimeleapCandidate,
    model: usize,
    max_ticks: usize,
) -> Vec<i64> {
    let mut status = candidate.post_status.clone();
    let mut cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: 0,
    };
    let mut sfmt = cached_sfmt_at(cache, candidate.post_sfmt_position).ok();
    let Some(ref mut sfmt) = sfmt else {
        return Vec::new();
    };
    let mut positions = Vec::new();
    for _ in 0..max_ticks {
        let blinks = advance_one_tick(&mut status, &mut cursor, sfmt);
        if blinks.iter().any(|blink| *blink as usize == model) {
            positions.push(cursor.sfmt_position);
        }
    }
    positions
}

/// ある候補のイベント後Timelineを指定Frameまで進め、対象モデルの瞬き位置を返す。
/// 候補ごとのTimelineは観測した時刻までだけ進め、設定されたFrame1候補幅とは混同しない。
#[cfg(test)]
pub(crate) fn predict_shift_times_until(
    cache: &ObservationCache,
    candidate: &TimeleapCandidate,
    model: usize,
    until_frame: i64,
) -> Vec<i64> {
    if until_frame < candidate.post_tick.saturating_mul(2) {
        return Vec::new();
    }
    let mut status = candidate.post_status.clone();
    let mut cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: candidate.post_tick,
    };
    let mut sfmt = cached_sfmt_at(cache, candidate.post_sfmt_position).ok();
    let Some(ref mut sfmt) = sfmt else {
        return Vec::new();
    };
    let mut times = Vec::new();
    for _ in 0..MAX_NORMAL_TIMELINE_TICKS {
        if cursor.tick.saturating_mul(2) > until_frame {
            break;
        }
        let blinks = advance_one_tick(&mut status, &mut cursor, sfmt);
        if blinks.iter().any(|blink| *blink as usize == model) {
            times.push(cursor.tick.saturating_mul(2));
        }
    }
    times
}

pub(super) fn apply_event_with_profile(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    mode: TimeleapMode,
    courier: &CourierGenerationProfile,
) {
    match mode {
        // 3DSRNGTool's Menu branch resets the fidget countdown; later ticks
        // perform the corresponding fidget RNG consumption in advance_one_tick.
        TimeleapMode::Menu => {
            status.fidget_cd = 3;
        }
        TimeleapMode::Dialogue => {
            advance_one_tick(status, cursor, sfmt);
            advance_one_tick(status, cursor, sfmt);
            sfmt.next_u64();
            cursor.sfmt_position += 1;
            cursor.frame += 1;
            advance_one_tick(status, cursor, sfmt);
        }
        TimeleapMode::Courier => {
            // Timeleap itself always follows the same event transition. The
            // No Dex / Your ID / egg condition belongs only to Event7 Consider
            // Delay, which is applied separately to candidate landing checks.
            for _ in 0..19 {
                advance_one_tick(status, cursor, sfmt);
            }
            let event_consumption = consume_courier_with_profile(sfmt, courier);
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(event_consumption);
            cursor.frame = cursor.frame.saturating_add(event_consumption);
            advance_one_tick(status, cursor, sfmt);
        }
    }
}

#[cfg(test)]
pub(super) fn apply_event(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    mode: TimeleapMode,
    courier: &CourierConfig,
) {
    apply_event_with_profile(status, cursor, sfmt, mode, &courier.generation_profile());
}

#[cfg(test)]
pub(super) fn consume_courier(sfmt: &mut Sfmt, config: &CourierConfig) -> i64 {
    consume_courier_with_profile(sfmt, &config.generation_profile())
}

fn consume_courier_with_profile(sfmt: &mut Sfmt, profile: &CourierGenerationProfile) -> i64 {
    consume_courier_with(profile, || Some(sfmt.next_u64()), None)
        .expect("SFMT always provides another output") as i64
}

/// 配達員生成が消費する乱数出力数を、供給された列から求める。
/// 候補窓の境界計算では、各候補位置の実際の可変IV抽選回数を評価する。
#[cfg(test)]
pub(super) fn courier_consumption_from_values(
    config: &CourierConfig,
    values: &[u64],
) -> Option<usize> {
    let mut values = values.iter().copied();
    consume_courier_with(&config.generation_profile(), || values.next(), None)
}

/// Exact first-hit Event7 generation bounds over every starting SFMT position.
/// IV generation consumes until it has drawn the requested number of distinct,
/// non-fixed IV slots. The accumulator visits the complete output sequence in
/// reverse once, retaining nearest future slot positions instead of replaying
/// the variable-consumption generator from each candidate start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CourierConsumptionBounds {
    /// Minimum complete generation length among starts that finish in the supplied range.
    pub(super) min: Option<usize>,
    /// Maximum complete generation length among starts that finish in the supplied range.
    pub(super) max: usize,
}

/// Accumulates exact first-hit generation bounds while visiting the complete
/// SFMT output sequence from right to left. It stores only the nearest future
/// occurrence of each of the six possible IV slots, so large searches need not
/// replay generation from every start or copy the whole SFMT range.
pub(super) struct CourierConsumptionBoundsAccumulator {
    prefix: usize,
    suffix: usize,
    eligible_slots: u8,
    required_slots: usize,
    sequence_len: usize,
    next_positions: [Option<usize>; 6],
    ordered_slots: [usize; 6],
    slot_ranks: [u8; 6],
    ordered_len: usize,
    bounds: CourierConsumptionBounds,
}

impl CourierConsumptionBoundsAccumulator {
    pub(super) fn new(config: &CourierConfig, sequence_len: usize) -> Self {
        let prefix = usize::from(config.ec.unwrap_or(0) == 0)
            .saturating_add(usize::from(config.pid_type != 3));
        let fixed_slots = config
            .fixed_ivs
            .iter()
            .enumerate()
            .fold(0u8, |slots, (index, value)| {
                if *value >= 0 {
                    slots | (1 << index)
                } else {
                    slots
                }
            });
        let eligible_slots = !fixed_slots & 0x3f;
        let required_slots = usize::from(config.iv_count);
        let available_slots = eligible_slots.count_ones() as usize;
        let suffix = available_slots
            .saturating_sub(required_slots)
            .saturating_add(usize::from(!config.ability_locked))
            .saturating_add(usize::from(!config.nature_locked))
            .saturating_add(usize::from(!config.gender_locked));

        let required_slots = if required_slots <= available_slots {
            required_slots
        } else {
            usize::MAX
        };

        Self {
            prefix,
            suffix,
            eligible_slots,
            required_slots,
            sequence_len,
            next_positions: [None; 6],
            ordered_slots: [0; 6],
            slot_ranks: [u8::MAX; 6],
            ordered_len: 0,
            bounds: CourierConsumptionBounds::default(),
        }
    }

    /// Add one raw SFMT output at its index while traversing the sequence
    /// backwards. Prefix outputs are not IV-slot draws for any valid start.
    pub(super) fn push_reverse(&mut self, index: usize, value: u64) {
        if index < self.prefix || self.required_slots == usize::MAX {
            return;
        }

        let slot = (value % 6) as usize;
        if self.eligible_slots & (1 << slot) != 0 {
            self.next_positions[slot] = Some(index);
            let previous_rank = self.slot_ranks[slot];
            if previous_rank != u8::MAX {
                let previous_rank = usize::from(previous_rank);
                for rank in previous_rank..self.ordered_len.saturating_sub(1) {
                    let moved_slot = self.ordered_slots[rank + 1];
                    self.ordered_slots[rank] = moved_slot;
                    self.slot_ranks[moved_slot] = rank as u8;
                }
                self.ordered_len -= 1;
            }
            for rank in (0..self.ordered_len).rev() {
                let moved_slot = self.ordered_slots[rank];
                self.ordered_slots[rank + 1] = moved_slot;
                self.slot_ranks[moved_slot] = (rank + 1) as u8;
            }
            self.ordered_slots[0] = slot;
            self.slot_ranks[slot] = 0;
            self.ordered_len += 1;
        }

        let start = index - self.prefix;
        let remaining = self.sequence_len.saturating_sub(start);
        let variable_draws = if self.required_slots == 0 {
            Some(0)
        } else {
            (self.ordered_len >= self.required_slots).then(|| {
                self.next_positions[self.ordered_slots[self.required_slots - 1]]
                    .expect("ordered IV slots have a next position")
                    .saturating_sub(index)
                    + 1
            })
        };

        let Some(variable_draws) = variable_draws else {
            return;
        };
        let total = self
            .prefix
            .saturating_add(variable_draws)
            .saturating_add(self.suffix);
        if total > remaining {
            return;
        }

        self.bounds.min = Some(self.bounds.min.map_or(total, |current| current.min(total)));
        self.bounds.max = self.bounds.max.max(total);
    }

    pub(super) fn finish(self) -> CourierConsumptionBounds {
        self.bounds
    }
}

#[cfg(test)]
pub(super) fn courier_consumption_bounds_from_values_cancelable(
    config: &CourierConfig,
    values: &[u64],
    cancel: Option<&AtomicBool>,
) -> Result<CourierConsumptionBounds, ()> {
    let mut accumulator = CourierConsumptionBoundsAccumulator::new(config, values.len());
    for index in (0..values.len()).rev() {
        if index % COURIER_CONSUMPTION_CANCEL_INTERVAL == 0 && is_cancelled(cancel) {
            return Err(());
        }
        accumulator.push_reverse(index, values[index]);
    }
    if is_cancelled(cancel) {
        return Err(());
    }
    Ok(accumulator.finish())
}

fn consume_courier_with(
    config: &CourierGenerationProfile,
    mut next_value: impl FnMut() -> Option<u64>,
    cancel: Option<&AtomicBool>,
) -> Option<usize> {
    let mut used = 0usize;
    let mut consume_next = || {
        if used % 16 == 0 && is_cancelled(cancel) {
            return None;
        }
        let value = next_value()?;
        used = used.saturating_add(1);
        Some(value)
    };
    // Event7 uses EC=0 as the random-generation sentinel, just like an unset field.
    if config.ec.unwrap_or(0) == 0 {
        consume_next()?;
    }
    if config.pid_type != 3 {
        consume_next()?;
    }
    let mut assigned_iv_slots =
        config
            .fixed_ivs
            .iter()
            .enumerate()
            .fold(0u8, |slots, (index, value)| {
                if *value >= 0 {
                    slots | (1 << index)
                } else {
                    slots
                }
            });
    let mut remaining = config.iv_count;
    while remaining > 0 {
        let index = (consume_next()? % 6) as usize;
        let slot = 1 << index;
        if assigned_iv_slots & slot == 0 {
            assigned_iv_slots |= slot;
            remaining -= 1;
        }
    }
    for index in 0..6 {
        if assigned_iv_slots & (1 << index) == 0 {
            consume_next()?;
        }
    }
    if !config.ability_locked {
        consume_next()?;
    }
    if !config.nature_locked {
        consume_next()?;
    }
    if !config.gender_locked {
        consume_next()?;
    }
    Some(used)
}

pub(super) fn advance_to_target(
    target: i64,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
) -> bool {
    for _ in 0..MAX_CANDIDATE_REPLAY_TICKS {
        if cursor.frame == target {
            return true;
        }
        if cursor.frame > target {
            return false;
        }
        advance_one_tick(status, cursor, sfmt);
    }
    false
}

/// A入力Frameを1つに早期確定せず、指定した生成開始Frameへ到達する全入力点を返す。
/// Frame1候補のTimelineは共有できるが、A入力位置ごとのConsider Delay結果は保持する。
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(super) fn apply_consider_delay(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    target_frame: i64,
) -> Result<bool, String> {
    apply_consider_delay_cancelable(
        config,
        status,
        cursor,
        sfmt,
        original_models,
        target_frame,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(super) fn apply_consider_delay_traced(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    target_frame: i64,
) -> Result<bool, String> {
    Ok(consider_delay_landing_frame_traced(
        config,
        status,
        cursor,
        sfmt,
        original_models,
        target_frame,
    )? == Some(target_frame))
}

pub(super) fn apply_consider_delay_cancelable(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    target_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    Ok(consider_delay_landing_frame_cancelable(
        config,
        status,
        cursor,
        sfmt,
        original_models,
        target_frame,
        cancel,
    )? == Some(target_frame))
}

pub(super) fn apply_event_consider_delay_from_values(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    original_models: usize,
    target_frame: i64,
    values_start: i64,
    values: &[u64],
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    let ConsiderDelayGenerator::Event = config.generator else {
        return Err("SFMT値ウィンドウ経路はEvent7専用です。".into());
    };
    if is_cancelled(cancel) {
        return Err("Timeleap候補検索がキャンセルされました。".into());
    }
    let delay_time = i32::try_from(config.delay_frames / 2 + 2)
        .map_err(|_| "Consider Delayの値が計算可能な範囲を超えています。")?;
    if !advance_event_delay_ticks_from_values(
        status,
        cursor,
        2,
        original_models,
        target_frame,
        values_start,
        values,
        cancel,
    )? {
        return Ok(false);
    }

    if config.courier.generates_during_event_delay()
        && consume_courier_until_from_values(
            cursor,
            &config.courier,
            target_frame,
            values_start,
            values,
            cancel,
        )?
        .is_none()
    {
        return Ok(false);
    }
    if cursor.frame > target_frame {
        return Ok(false);
    }
    let finished = advance_event_delay_ticks_from_values(
        status,
        cursor,
        delay_time.saturating_sub(2),
        original_models,
        target_frame,
        values_start,
        values,
        cancel,
    )?;
    Ok(finished && cursor.frame == target_frame)
}

fn consume_courier_until_from_values(
    cursor: &mut Cursor,
    config: &CourierConfig,
    target_frame: i64,
    values_start: i64,
    values: &[u64],
    cancel: Option<&AtomicBool>,
) -> Result<Option<usize>, String> {
    let mut missing_value = false;
    let consumed = consume_courier_with(
        &config.generation_profile(),
        || {
            if cursor.frame >= target_frame || is_cancelled(cancel) {
                return None;
            }
            let Some(value) = cached_sfmt_value(values_start, values, cursor.sfmt_position) else {
                missing_value = true;
                return None;
            };
            cursor.frame = cursor.frame.saturating_add(1);
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            Some(value)
        },
        cancel,
    );
    if is_cancelled(cancel) {
        return Err("Timeleap候補検索がキャンセルされました。".into());
    }
    if missing_value {
        return Err("Event7計算に必要な共有SFMT値が範囲外です。".into());
    }
    Ok(consumed)
}

fn advance_event_delay_ticks_from_values(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    ticks: i32,
    original_models: usize,
    target_frame: i64,
    values_start: i64,
    values: &[u64],
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    ensure_model_capacity(status, original_models);
    let mut remaining_ticks = usize::try_from(ticks.max(0)).unwrap_or(0);
    while remaining_ticks > 0 {
        if is_cancelled(cancel) {
            return Err("Timeleap候補検索がキャンセルされました。".into());
        }
        let skipped = advance_no_blink_run_from_values(
            status,
            cursor,
            target_frame,
            remaining_ticks,
            values_start,
            values,
            false,
        );
        if skipped > 0 {
            remaining_ticks -= skipped;
            if cursor.frame > target_frame {
                return Ok(false);
            }
            continue;
        }
        let active_models = status.model.remain.len();
        if !advance_one_tick_state_only_from_values(
            status,
            cursor,
            active_models,
            false,
            values_start,
            values,
        ) {
            return Err("Event7計算に必要な共有SFMT値が範囲外です。".into());
        }
        remaining_ticks -= 1;
        if cursor.frame > target_frame {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Advance through consecutive ticks that cannot change the RNG state.
/// Active 0/1 models are checked against the cached SFMT stream; negative
/// cooldowns and positive waits are advanced only up to their next RNG boundary.
pub(super) fn advance_no_blink_run_from_values(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    target: i64,
    remaining_limit: usize,
    values_start: i64,
    values: &[u64],
    advance_fidget: bool,
) -> usize {
    advance_no_blink_run_from_values_with_index(
        status,
        cursor,
        target,
        remaining_limit,
        values_start,
        values,
        advance_fidget,
        None,
    )
}

/// One immutable index for repeatedly searched cached SFMT windows. Each entry
/// stores the absolute array offset of the next value that starts a blink.
pub(super) struct NoBlinkTriggerIndex {
    next_trigger: Vec<u32>,
}

impl NoBlinkTriggerIndex {
    pub(super) fn new(values: &[u64]) -> Self {
        let mut next = u32::MAX;
        let mut next_trigger = vec![u32::MAX; values.len()];
        for index in (0..values.len()).rev() {
            if values[index] & 0x7f == 0 {
                next = u32::try_from(index).expect("cached SFMT window fits u32");
            }
            next_trigger[index] = next;
        }
        Self { next_trigger }
    }

    fn next_trigger_at_or_after(&self, offset: usize) -> Option<usize> {
        self.next_trigger
            .get(offset)
            .copied()
            .filter(|index| *index != u32::MAX)
            .and_then(|index| usize::try_from(index).ok())
    }
}

pub(super) fn advance_no_blink_run_from_values_with_index(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    target: i64,
    remaining_limit: usize,
    values_start: i64,
    values: &[u64],
    advance_fidget: bool,
    trigger_index: Option<&NoBlinkTriggerIndex>,
) -> usize {
    if status.raining
        || status.model.remain.is_empty()
        || (advance_fidget && status.fidget_cd == 1)
        || cursor.frame >= target
        || remaining_limit == 0
    {
        return 0;
    }

    let active_models = status
        .model
        .remain
        .iter()
        .filter(|remain| matches!(**remain, 0 | 1))
        .count();
    let has_negative_cooldown = status.model.remain.iter().any(|remain| *remain < 0);
    if active_models == 0 && !has_negative_cooldown {
        return 0;
    }

    let frames_per_tick = active_models as i64;
    let mut limit = remaining_limit;
    for remain in &status.model.remain {
        if *remain > 1 {
            limit = limit.min(usize::try_from(remain - 1).unwrap_or(0));
        } else if *remain < 0 {
            // Stop at -1: the following tick resolves the cooldown and consumes RNG.
            let ticks_before_rng = (-1_i32)
                .saturating_sub(*remain)
                .try_into()
                .unwrap_or(usize::MAX);
            limit = limit.min(ticks_before_rng);
        }
    }
    if advance_fidget && status.fidget_cd > 1 {
        limit = limit.min(usize::try_from(status.fidget_cd - 1).unwrap_or(0));
    }

    if active_models > 0 {
        let distance = target.saturating_sub(cursor.frame);
        let ticks_until_target = distance
            .saturating_add(frames_per_tick.saturating_sub(1))
            .checked_div(frames_per_tick)
            .and_then(|ticks| usize::try_from(ticks).ok())
            .unwrap_or(remaining_limit);
        limit = limit.min(ticks_until_target);
        limit = limit.min(MAX_NO_BLINK_SCAN_OUTPUTS / active_models);
    } else {
        // No model consumes RNG until a negative cooldown resolves.
        limit = limit.min(MAX_NO_BLINK_SCAN_OUTPUTS);
    }
    if limit == 0 {
        return 0;
    }

    let Some(offset) = cursor
        .sfmt_position
        .checked_sub(values_start)
        .and_then(|offset| usize::try_from(offset).ok())
    else {
        return 0;
    };
    let safe_ticks = if active_models > 0 {
        let available_ticks = values.len().saturating_sub(offset) / active_models;
        limit = limit.min(available_ticks);
        let values_to_scan = limit.saturating_mul(active_models);
        if let Some(trigger_index) = trigger_index {
            trigger_index
                .next_trigger_at_or_after(offset)
                .filter(|trigger| trigger.saturating_sub(offset) < values_to_scan)
                .map(|trigger| trigger.saturating_sub(offset) / active_models)
                .unwrap_or(limit)
        } else {
            let Some(run_values) = values.get(offset..offset.saturating_add(values_to_scan)) else {
                return 0;
            };
            run_values
                .iter()
                .position(|value| value & 0x7f == 0)
                .map(|first_trigger_output| first_trigger_output / active_models)
                .unwrap_or(limit)
        }
    } else {
        limit
    };
    if safe_ticks == 0 {
        return 0;
    }

    let decrement = i32::try_from(safe_ticks).expect("bounded model run fits i32");
    if advance_fidget && status.fidget_cd > 1 {
        status.fidget_cd -= decrement;
    }
    for remain in &mut status.model.remain {
        if *remain > 1 {
            *remain -= decrement;
        } else if *remain < 0 {
            *remain += decrement;
        }
    }
    let tick_count = safe_ticks as i64;
    let outputs = tick_count.saturating_mul(frames_per_tick);
    cursor.frame = cursor.frame.saturating_add(outputs);
    cursor.sfmt_position = cursor.sfmt_position.saturating_add(outputs);
    cursor.tick = cursor.tick.saturating_add(tick_count);
    safe_ticks
}

fn cached_sfmt_value(values_start: i64, values: &[u64], position: i64) -> Option<u64> {
    let offset = usize::try_from(position.checked_sub(values_start)?).ok()?;
    values.get(offset).copied()
}

#[cfg(test)]
pub(super) fn consider_delay_landing_frame(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    max_frame: i64,
) -> Result<Option<i64>, String> {
    consider_delay_landing_frame_cancelable(
        config,
        status,
        cursor,
        sfmt,
        original_models,
        max_frame,
        None,
    )
}

pub(super) fn consider_delay_landing_frame_cancelable(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    max_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<Option<i64>, String> {
    consider_delay_landing_frame_cancelable_with_trace::<false>(
        config,
        status,
        cursor,
        sfmt,
        original_models,
        max_frame,
        cancel,
    )
}

#[cfg(test)]
pub(super) fn consider_delay_landing_frame_traced(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    max_frame: i64,
) -> Result<Option<i64>, String> {
    consider_delay_landing_frame_cancelable_with_trace::<true>(
        config,
        status,
        cursor,
        sfmt,
        original_models,
        max_frame,
        None,
    )
}

fn consider_delay_landing_frame_cancelable_with_trace<const TRACE_RNG: bool>(
    config: &ConsiderDelayConfig,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    max_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<Option<i64>, String> {
    if is_cancelled(cancel) {
        return Err("Timeleap候補検索がキャンセルされました。".into());
    }
    let delay_time = i32::try_from(config.delay_frames / 2 + 2)
        .map_err(|_| "Consider Delayの値が計算可能な範囲を超えています。")?;
    let reached = match config.generator {
        ConsiderDelayGenerator::Event => {
            advance_event_delay_ticks_with_trace::<TRACE_RNG>(
                status,
                cursor,
                sfmt,
                2,
                original_models,
                max_frame,
                cancel,
            );
            if is_cancelled(cancel) {
                return Err("Timeleap候補検索がキャンセルされました。".into());
            }
            if cursor.frame > max_frame {
                false
            } else {
                let generate = config.courier.generates_during_event_delay();
                if generate
                    && consume_courier_until(sfmt, cursor, &config.courier, max_frame, cancel)
                        .is_none()
                {
                    if is_cancelled(cancel) {
                        return Err("Timeleap候補検索がキャンセルされました。".into());
                    }
                    return Ok(None);
                }
                if cursor.frame <= max_frame {
                    let reached = advance_event_delay_ticks_with_trace::<TRACE_RNG>(
                        status,
                        cursor,
                        sfmt,
                        delay_time.saturating_sub(2),
                        original_models,
                        max_frame,
                        cancel,
                    );
                    if is_cancelled(cancel) {
                        return Err("Timeleap候補検索がキャンセルされました。".into());
                    }
                    reached
                } else {
                    false
                }
            }
        }
        ConsiderDelayGenerator::FixedSymbol(_) => {
            let type_id = consider_delay_type_id(config);
            apply_stationary_delay_with_trace::<TRACE_RNG>(
                type_id,
                delay_time,
                status,
                cursor,
                sfmt,
                original_models,
                max_frame,
            )
        }
    };
    Ok(reached.then_some(cursor.frame))
}

pub(super) fn consume_courier_until(
    sfmt: &mut Sfmt,
    cursor: &mut Cursor,
    config: &CourierConfig,
    target_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Option<i64> {
    let initial_frame = cursor.frame;
    let mut next = || {
        if cursor.frame >= target_frame || is_cancelled(cancel) {
            return None;
        }
        let value = sfmt.next_u64();
        cursor.frame = cursor.frame.saturating_add(1);
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        Some(value)
    };
    if config.ec.unwrap_or(0) == 0 {
        next()?;
    }
    if config.pid_type != 3 {
        next()?;
    }
    let mut fixed = config.fixed_ivs;
    let mut remaining = config.iv_count;
    while remaining > 0 {
        let index = (next()? % 6) as usize;
        if fixed[index] < 0 {
            fixed[index] = 31;
            remaining -= 1;
        }
    }
    for value in fixed {
        if value < 0 {
            next()?;
        }
    }
    if !config.ability_locked {
        next()?;
    }
    if !config.nature_locked {
        next()?;
    }
    if !config.gender_locked {
        next()?;
    }
    Some(cursor.frame.saturating_sub(initial_frame))
}

pub(super) fn consider_delay_type_id(config: &ConsiderDelayConfig) -> i32 {
    match config.generator {
        ConsiderDelayGenerator::FixedSymbol(delay_type) => {
            if delay_type == crate::domain::fixed_rng_catalog::StationaryDelayType::CommonLegendary
                && config.delay_frames % 2 != 0
            {
                // 3DSRNGTool maps odd raw delays for DelayType 4 to its DelayType 6 route.
                6
            } else {
                delay_type as u8 as i32
            }
        }
        ConsiderDelayGenerator::Event => 0,
    }
}

pub(super) fn delay_elapsed_tick_bound(config: &ConsiderDelayConfig, delay_ticks: i32) -> i32 {
    match config.generator {
        ConsiderDelayGenerator::Event => delay_ticks.max(2),
        ConsiderDelayGenerator::FixedSymbol(_) => {
            let minimum = match consider_delay_type_id(config) {
                4 => 2,
                6 => 27,
                1 => 98,
                2 => 95,
                3 => 45,
                5 => 2,
                9 => 77,
                10 => 74,
                11 | 21 => 36,
                12 => 51,
                13 | 23 => 64,
                14 => 46,
                15 => 50,
                16 => 43,
                17 => 40,
                22 => 376,
                24 => 412,
                25 => 392,
                26 => 746,
                27 => 337,
                _ => 0,
            };
            delay_ticks.max(minimum)
        }
    }
}

pub(super) fn advance_random(cursor: &mut Cursor, sfmt: &mut Sfmt, count: i64) {
    for _ in 0..count.max(0) {
        sfmt.next_u64();
        cursor.frame = cursor.frame.saturating_add(1);
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
    }
}

fn advance_event_delay_ticks_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    ticks: i32,
    original_models: usize,
    target_frame: i64,
    cancel: Option<&AtomicBool>,
) -> bool {
    for index in 0..ticks.max(0) {
        if index % 32 == 0 && is_cancelled(cancel) {
            return false;
        }
        ensure_model_capacity(status, original_models);
        let active_models = status.model.remain.len();
        advance_one_event_delay_tick_with_trace::<TRACE_RNG>(status, cursor, sfmt, active_models);
        if cursor.frame > target_frame {
            return false;
        }
    }
    true
}

#[cfg(test)]
fn advance_one_event_delay_tick(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
) {
    advance_one_event_delay_tick_with_trace::<false>(status, cursor, sfmt, active_models);
}

fn advance_one_event_delay_tick_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
) {
    ensure_model_capacity(status, active_models);
    let mut next_model_value = || {
        #[cfg(test)]
        let position = cursor.sfmt_position;
        let value = sfmt.next_u64();
        cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
        #[cfg(test)]
        {
            (position, value)
        }
        #[cfg(not(test))]
        {
            let _ = TRACE_RNG;
            value
        }
    };
    #[cfg(test)]
    let model_outputs = if TRACE_RNG {
        status
            .model
            .next_state_prefix_with_observer_without_blink_list(
                active_models,
                &mut next_model_value,
                |draw, remain_after| record_model_rng_event(cursor.tick, draw, remain_after),
            )
    } else {
        let mut next_value = || {
            let (_, value) = next_model_value();
            value
        };
        status
            .model
            .next_state_prefix_without_blink_list(active_models, &mut next_value)
    };
    #[cfg(not(test))]
    let model_outputs = {
        let mut next_value = || next_model_value();
        status
            .model
            .next_state_prefix_without_blink_list(active_models, &mut next_value)
    };
    cursor.frame = cursor.frame.saturating_add(model_outputs as i64);
    if status.raining {
        status.phase = (status.phase + 1) % 2;
        advance_random(cursor, sfmt, 1);
    }
    cursor.tick = cursor.tick.saturating_add(1);
}

pub(super) fn ensure_model_capacity(status: &mut TimeleapStatus, models: usize) {
    if status.model.remain.len() < models {
        status.model.remain.resize(models, 0);
    }
}

pub(super) fn change_active_model_count(
    active_models: &mut usize,
    new_count: usize,
    status: &mut TimeleapStatus,
) {
    ensure_model_capacity(status, new_count);
    *active_models = new_count;
}

fn advance_stationary_ticks_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    ticks: i32,
    active_models: usize,
    target_frame: i64,
) -> bool {
    for _ in 0..ticks.max(0) {
        if !advance_delay_tick_with_trace::<TRACE_RNG>(status, cursor, sfmt, active_models)
            || cursor.frame > target_frame
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
pub(super) fn advance_delay_tick(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
) -> bool {
    advance_delay_tick_with_trace::<false>(status, cursor, sfmt, active_models)
}

fn advance_delay_tick_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
) -> bool {
    ensure_model_capacity(status, active_models);
    // Fixed-symbol Consider Delay follows RNGPool.time_elapse7: it advances
    // model state but does not process menu fidget or consume its RNG values.
    advance_one_tick_state_only_with_trace::<TRACE_RNG>(status, cursor, sfmt, active_models, false);
    true
}

#[cfg(test)]
pub(super) fn advance_delay_tick_reference(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
) -> bool {
    ensure_model_capacity(status, active_models);
    let inactive = status.model.remain.split_off(active_models);
    advance_one_tick_reference_with_fidget_policy(status, cursor, sfmt, false);
    status.model.remain.extend(inactive);
    true
}

fn cry_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    active_models: usize,
    cry_index: usize,
) {
    ensure_model_capacity(status, active_models);
    for model in 0..active_models {
        if model == cry_index {
            advance_random(cursor, sfmt, 1);
        }
        let remain = &mut status.model.remain[model];
        if *remain > 1 {
            *remain -= 1;
        } else if *remain < 0 {
            *remain += 1;
            if *remain == 0 {
                #[cfg(test)]
                let position = cursor.sfmt_position;
                let value = sfmt.next_u64();
                *remain = if value % 3 == 0 { 36 } else { 30 };
                #[cfg(test)]
                if TRACE_RNG {
                    record_model_rng_events(
                        cursor.tick,
                        &[ModelConsumption {
                            model,
                            position,
                            value,
                            blink: true,
                        }],
                        &status.model.remain,
                    );
                }
                cursor.frame = cursor.frame.saturating_add(1);
                cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            }
        } else {
            #[cfg(test)]
            let position = cursor.sfmt_position;
            let value = sfmt.next_u64();
            cursor.frame = cursor.frame.saturating_add(1);
            cursor.sfmt_position = cursor.sfmt_position.saturating_add(1);
            if value & 0x7f == 0 {
                *remain = -5;
            }
            #[cfg(test)]
            if TRACE_RNG {
                record_model_rng_events(
                    cursor.tick,
                    &[ModelConsumption {
                        model,
                        position,
                        value,
                        blink: false,
                    }],
                    &status.model.remain,
                );
            }
        }
    }
    if cry_index >= active_models {
        advance_random(cursor, sfmt, 1);
    }
    #[cfg(not(test))]
    let _ = TRACE_RNG;
    cursor.tick = cursor.tick.saturating_add(1);
}

fn split_delay_with_trace<const TRACE_RNG: bool>(
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    total_delay: i32,
    cry_delay: i32,
    active_models: usize,
    target_frame: i64,
) -> bool {
    if !advance_stationary_ticks_with_trace::<TRACE_RNG>(
        status,
        cursor,
        sfmt,
        total_delay.saturating_sub(cry_delay),
        active_models,
        target_frame,
    ) {
        return false;
    }
    advance_random(cursor, sfmt, 1);
    advance_stationary_ticks_with_trace::<TRACE_RNG>(
        status,
        cursor,
        sfmt,
        cry_delay,
        active_models,
        target_frame,
    )
}

#[cfg(test)]
pub(super) fn apply_stationary_delay(
    delay_type: i32,
    delay_time: i32,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    target_frame: i64,
) -> bool {
    apply_stationary_delay_with_trace::<false>(
        delay_type,
        delay_time,
        status,
        cursor,
        sfmt,
        original_models,
        target_frame,
    )
}

fn apply_stationary_delay_with_trace<const TRACE_RNG: bool>(
    delay_type: i32,
    delay_time: i32,
    status: &mut TimeleapStatus,
    cursor: &mut Cursor,
    sfmt: &mut Sfmt,
    original_models: usize,
    target_frame: i64,
) -> bool {
    let mut active_models = original_models;
    let run = |ticks,
               active_models,
               status: &mut TimeleapStatus,
               cursor: &mut Cursor,
               sfmt: &mut Sfmt| {
        advance_stationary_ticks_with_trace::<TRACE_RNG>(
            status,
            cursor,
            sfmt,
            ticks,
            active_models,
            target_frame,
        )
    };
    let split = |status: &mut TimeleapStatus,
                 cursor: &mut Cursor,
                 sfmt: &mut Sfmt,
                 total_delay,
                 cry_delay,
                 active_models,
                 target_frame| {
        split_delay_with_trace::<TRACE_RNG>(
            status,
            cursor,
            sfmt,
            total_delay,
            cry_delay,
            active_models,
            target_frame,
        )
    };
    let cry = |status: &mut TimeleapStatus,
               cursor: &mut Cursor,
               sfmt: &mut Sfmt,
               active_models,
               cry_index| {
        cry_with_trace::<TRACE_RNG>(status, cursor, sfmt, active_models, cry_index)
    };
    match delay_type {
        1 | 2 => {
            let cry_delay = if delay_type == 1 { 78 } else { 75 };
            if !run(
                delay_time - cry_delay - 20,
                active_models,
                status,
                cursor,
                sfmt,
            ) {
                return false;
            }
            if original_models == 7 {
                let previous = status.model.remain.clone();
                for (destination, source) in [0, 1, 2, 5, 6].into_iter().enumerate() {
                    status.model.remain[destination] = previous[source];
                }
                active_models = 5;
            }
            if !run(19, active_models, status, cursor, sfmt) {
                return false;
            }
            cry(status, cursor, sfmt, active_models, 3);
            run(cry_delay, active_models, status, cursor, sfmt)
        }
        3 => {
            if !run(3, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            split(
                status,
                cursor,
                sfmt,
                delay_time - 3,
                delay_time - 45,
                active_models,
                target_frame,
            )
        }
        5 => {
            if !run(delay_time - 2, active_models, status, cursor, sfmt) {
                return false;
            }
            for index in [1, 3, 4, 5] {
                if let Some(remain) = status.model.remain.get_mut(index) {
                    *remain = 0;
                }
            }
            run(2, active_models, status, cursor, sfmt)
        }
        4 => {
            advance_random(cursor, sfmt, 2);
            run(delay_time - 2, active_models, status, cursor, sfmt) && {
                change_active_model_count(&mut active_models, 1, status);
                run(2, active_models, status, cursor, sfmt)
            }
        }
        6 => {
            advance_random(cursor, sfmt, 2);
            if !run(delay_time - 27, active_models, status, cursor, sfmt) {
                return false;
            }
            advance_random(cursor, sfmt, 1);
            if !run(25, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 1, status);
            run(2, active_models, status, cursor, sfmt)
        }
        9 => {
            if !run(delay_time - 77, active_models, status, cursor, sfmt) {
                return false;
            }
            cry(status, cursor, sfmt, active_models, 8);
            run(76, active_models, status, cursor, sfmt)
        }
        10 => {
            if !run(delay_time - 74, active_models, status, cursor, sfmt) {
                return false;
            }
            if active_models == 9 {
                let previous = status.model.remain.clone();
                for (destination, source) in [0, 1, 2, 6, 7, 8].into_iter().enumerate() {
                    status.model.remain[destination] = previous[source];
                }
                active_models = 6;
            }
            split(status, cursor, sfmt, 74, 73, active_models, target_frame)
        }
        11 | 21 => split(
            status,
            cursor,
            sfmt,
            delay_time,
            36,
            active_models,
            target_frame,
        ),
        12 => split(
            status,
            cursor,
            sfmt,
            delay_time,
            51,
            active_models,
            target_frame,
        ),
        13 | 23 => {
            if !run(29, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 3, status);
            if !run(delay_time - 64, active_models, status, cursor, sfmt) {
                return false;
            }
            cry(status, cursor, sfmt, active_models, 2);
            run(34, active_models, status, cursor, sfmt)
        }
        14 => {
            if !run(11, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            split(
                status,
                cursor,
                sfmt,
                delay_time - 11,
                35,
                active_models,
                target_frame,
            )
        }
        15 => split(
            status,
            cursor,
            sfmt,
            delay_time,
            50,
            active_models,
            target_frame,
        ),
        16 => split(
            status,
            cursor,
            sfmt,
            delay_time,
            43,
            active_models,
            target_frame,
        ),
        17 => split(
            status,
            cursor,
            sfmt,
            delay_time,
            40,
            active_models,
            target_frame,
        ),
        22 => {
            advance_random(cursor, sfmt, 2);
            if !run(delay_time - 376, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 1, status);
            if !run(214, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            split(status, cursor, sfmt, 162, 51, active_models, target_frame)
        }
        24 => {
            if !run(delay_time - 412, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            if !run(42, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 1, status);
            if !run(93, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            split(status, cursor, sfmt, 277, 35, active_models, target_frame)
        }
        25 => {
            if !run(66, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 1, status);
            if !run(34, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            if !run(165, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 1, status);
            if !run(77, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            split(
                status,
                cursor,
                sfmt,
                delay_time - 342,
                50,
                active_models,
                target_frame,
            )
        }
        26 => {
            for (ticks, models) in [(77, 1), (34, 2), (91, 3), (61, 2), (369, 4)] {
                if !run(ticks, active_models, status, cursor, sfmt) {
                    return false;
                }
                change_active_model_count(&mut active_models, models, status);
            }
            status.model.remain[3] = status.model.remain[2];
            status.model.remain[2] = 0;
            if !run(71, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            split(
                status,
                cursor,
                sfmt,
                delay_time - 703,
                43,
                active_models,
                target_frame,
            )
        }
        27 => {
            if !run(196, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 1, status);
            if !run(101, active_models, status, cursor, sfmt) {
                return false;
            }
            change_active_model_count(&mut active_models, 2, status);
            split(
                status,
                cursor,
                sfmt,
                delay_time - 297,
                40,
                active_models,
                target_frame,
            )
        }
        _ => run(delay_time, active_models, status, cursor, sfmt),
    }
}
