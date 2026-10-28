use super::super::rng::{ObservationCache, Sfmt};
use super::candidate_search::{
    consider_delay_initial_window_frames, is_cancelled, max_event_generation_consumption,
    search_consider_delay_inputs, Cursor, TimeleapSearchStats, TimeleapStatus,
};
use super::run_timeleap_frame1_preparation;
use super::runtime::{
    advance_one_tick, cached_sfmt_at, consider_delay_landing_frame_cancelable,
    delay_elapsed_tick_bound, validate_config,
};
use super::{
    ConfirmedTargetSearchRequest, ConfirmedTargetSearchRow, ConfirmedTimeleapStart, TimeleapConfig,
    TimeleapMode, TimeleapReachability, TimeleapReachabilityRequest, TimeleapSearchFailure,
    TimeleapTimelineStart, MAX_CANDIDATE_REPLAY_TICKS,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

#[cfg(test)]
use super::{
    timeleap_target_reachable_after_frame1, ConfirmedTargetSearchResult, ConsiderDelayConfig,
    ConsiderDelayGenerator, CourierConfig,
};

#[cfg(test)]
mod tests;

/// Compute the visual position for a drift-search marker once per accepted result.
/// `None` means the requested frame is present exactly; otherwise the returned index
/// is the boundary before that row (or after the final row).
pub(crate) fn confirmed_target_marker_boundary(
    rows: &[ConfirmedTargetSearchRow],
    requested: i64,
) -> Option<usize> {
    if rows.is_empty()
        || rows
            .iter()
            .any(|row| row.generation_start_frame == Some(requested))
    {
        return None;
    }

    let bracket = rows
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| {
            match (
                pair[0].generation_start_frame,
                pair[1].generation_start_frame,
            ) {
                (Some(first), Some(second)) => {
                    (first < requested && requested < second)
                        || (second < requested && requested < first)
                }
                _ => false,
            }
        })
        .min_by_key(|(_, pair)| {
            pair.iter()
                .map(|row| row.drift_frames.saturating_abs())
                .min()
                .unwrap_or(i64::MAX)
        })
        .map(|(index, _)| index + 1);
    if bracket.is_some() {
        return bracket;
    }

    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            row.generation_start_frame.map(|frame| {
                (
                    (
                        frame.saturating_sub(requested).saturating_abs(),
                        row.drift_frames.saturating_abs(),
                    ),
                    index,
                    frame,
                )
            })
        })
        .min_by_key(|(distance, _, _)| *distance)
        .map(
            |(_, index, frame)| {
                if requested > frame {
                    index + 1
                } else {
                    index
                }
            },
        )
}

fn assign_confirmed_target_grace_frames(rows: &mut [ConfirmedTargetSearchRow]) {
    let mut index = 0;
    while index < rows.len() {
        let Some(generation_start_frame) = rows[index].generation_start_frame else {
            index += 1;
            continue;
        };

        let mut end = index + 1;
        while end < rows.len()
            && rows[end].generation_start_frame == Some(generation_start_frame)
            && rows[end].target_frame > rows[end - 1].target_frame
        {
            end += 1;
        }

        let grace_frames = (end - index).saturating_mul(2) as i64;
        for row in &mut rows[index..end] {
            row.grace_frames = grace_frames;
        }
        index = end;
    }
}

pub(crate) fn run_timeleap_reachability_check(
    request: &TimeleapReachabilityRequest,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<TimeleapReachability, TimeleapSearchFailure> {
    let config = &request.config;
    validate_config(config).map_err(TimeleapSearchFailure::Calculation)?;
    if cache.seed != config.seed {
        return Err(TimeleapSearchFailure::Calculation(
            "入力SeedとSFMTキャッシュが一致しません。".into(),
        ));
    }
    if request.frame1 < config.start_frame || request.frame1 > config.target_frame {
        return Ok(TimeleapReachability::Frame1Unreachable);
    }

    let delay_headroom = request.consider_delay.as_ref().map_or(0_i64, |delay| {
        let delay_ticks = i32::try_from(delay.delay_frames.max(0) / 2 + 2).unwrap_or(i32::MAX);
        let ordinary_window = consider_delay_initial_window_frames(
            delay,
            delay_ticks,
            config,
            super::candidate_search::EventGenerationFrameBounds::default(),
            false,
        )
        .0;
        i64::from(delay_elapsed_tick_bound(delay, delay_ticks))
            .saturating_mul(config.models.saturating_add(4) as i64)
            .max(ordinary_window)
    });
    let base_required_end = config
        .target_frame
        .saturating_add(delay_headroom)
        .saturating_add(311)
        .min(crate::domain::rng::MAX_SFMT_FRAME);
    cache
        .ensure_through(base_required_end, Some(cancel))
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
    let event_generation_bounds = if let Some(delay) = request.consider_delay.as_ref() {
        max_event_generation_consumption(
            cache,
            delay,
            config,
            Cursor {
                frame: config.start_frame,
                sfmt_position: config.start_frame,
                tick: 0,
            },
            Some(cancel),
        )
        .map_err(TimeleapSearchFailure::Calculation)?
    } else {
        super::candidate_search::EventGenerationFrameBounds::default()
    };
    let required_end = base_required_end
        .saturating_add(event_generation_bounds.max)
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

    // A zero-width preparation produces the exact ordinary-Timeline state at
    // the entered Frame1, then applies the selected event once.
    let mut frame1_config = config.clone();
    frame1_config.handoff_range_frames = 0;
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);
    let preparation = match run_timeleap_frame1_preparation(
        &frame1_config,
        request.frame1,
        &start,
        cache,
        cancel,
    ) {
        Ok(preparation) => preparation,
        Err(TimeleapSearchFailure::Frame1Unreachable) => {
            return Ok(TimeleapReachability::Frame1Unreachable);
        }
        Err(error) => return Err(error),
    };
    let Some(candidate) = preparation
        .candidates
        .iter()
        .find(|candidate| candidate.frame1 == request.frame1)
    else {
        return Ok(TimeleapReachability::Frame1Unreachable);
    };

    let mut cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: candidate.post_tick,
    };
    if cursor.frame > config.target_frame {
        return Ok(TimeleapReachability::GenerationFrameUnreachable);
    }
    let mut status = candidate.post_status.clone();
    let mut sfmt =
        cached_sfmt_at(cache, cursor.sfmt_position).map_err(TimeleapSearchFailure::Calculation)?;

    let target_input_frames = if let Some(delay) = request.consider_delay.as_ref() {
        let delay_ticks = i32::try_from(delay.delay_frames / 2 + 2).map_err(|_| {
            TimeleapSearchFailure::Calculation("Consider Delayの値を確認してください。".into())
        })?;
        let mut suffixes = HashMap::new();
        let hits = search_consider_delay_inputs(
            delay,
            delay_ticks,
            config,
            &status,
            &cursor,
            &sfmt,
            cache,
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
        })?;
        hits.into_iter()
            .map(|hit| hit.input_cursor.frame)
            .collect::<Vec<_>>()
    } else {
        let mut ticks = 0usize;
        while cursor.frame < config.target_frame && ticks < MAX_CANDIDATE_REPLAY_TICKS {
            if is_cancelled(Some(cancel)) {
                return Err(TimeleapSearchFailure::Cancelled);
            }
            advance_one_tick(&mut status, &mut cursor, &mut sfmt);
            ticks += 1;
        }
        if cursor.frame == config.target_frame {
            vec![config.target_frame]
        } else {
            Vec::new()
        }
    };

    if target_input_frames.is_empty() {
        Ok(TimeleapReachability::GenerationFrameUnreachable)
    } else {
        Ok(TimeleapReachability::Reachable {
            target_input_frames,
        })
    }
}

/// Search the post-Frame1 Timeline for inputs that produce the requested
/// generation-start frame. The baseline TargetFrame is display-only.
pub(crate) fn run_confirmed_target_search(
    request: &ConfirmedTargetSearchRequest,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<Vec<ConfirmedTargetSearchRow>, TimeleapSearchFailure> {
    if cache.seed != request.seed {
        return Err(TimeleapSearchFailure::Calculation(
            "入力SeedとSFMTキャッシュが一致しません。".into(),
        ));
    }
    let models = request.source_config.as_ref().map_or_else(
        || request.start.status.model.remain.len(),
        |config| config.models,
    );
    if models == 0
        || !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&request.confirmed_frame1)
        || (request.source_config.is_none()
            && (request.start.sfmt_position < 0
                || request.start.frame < 0
                || request.start.tick < 0))
        || !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&request.baseline_target_frame)
        || !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&request.generation_start_frame)
        || !(0..=100_000).contains(&request.drift_range_ticks)
        || !(0..=98).contains(&request.grace_min_frames)
    {
        return Err(TimeleapSearchFailure::Calculation(
            "TargetFrame検索の入力範囲を確認してください。".into(),
        ));
    }
    if let Some(config) = request.source_config.as_ref() {
        validate_config(config).map_err(TimeleapSearchFailure::Calculation)?;
        if config.seed != request.seed || request.confirmed_frame1 < config.start_frame {
            return Ok(Vec::new());
        }
    }
    let start = if let Some(source_config) = request.source_config.as_ref() {
        let mut frame1_config = source_config.clone();
        frame1_config.handoff_range_frames = 0;
        let initial =
            TimeleapTimelineStart::initial(source_config.start_frame, source_config.models);
        let preparation = run_timeleap_frame1_preparation(
            &frame1_config,
            request.confirmed_frame1,
            &initial,
            cache,
            cancel,
        )?;
        let Some(candidate) = preparation
            .candidates
            .iter()
            .find(|candidate| candidate.frame1 == request.confirmed_frame1)
        else {
            return Ok(Vec::new());
        };
        ConfirmedTimeleapStart {
            frame: candidate.post_frame,
            sfmt_position: candidate.post_sfmt_position,
            tick: candidate.post_tick,
            status: candidate.post_status.clone(),
        }
    } else {
        request.start.clone()
    };
    // The baseline TargetFrame is only used to calculate the displayed drift;
    // it must not constrain which generation-start values are searched.
    confirmed_target_search_from_frame1(request, &start, models, cache, cancel)
}

fn confirmed_target_drift_frames(candidate_tick: i64, baseline_tick: i64) -> i64 {
    candidate_tick
        .saturating_sub(baseline_tick)
        .saturating_mul(2)
}

fn confirmed_target_search_from_frame1(
    request: &ConfirmedTargetSearchRequest,
    start: &ConfirmedTimeleapStart,
    models: usize,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<Vec<ConfirmedTargetSearchRow>, TimeleapSearchFailure> {
    let delay_tick_bound = request.consider_delay.as_ref().map_or(0, |delay| {
        (delay.delay_frames.max(0) / 2 + 2).min(i64::from(i32::MAX)) as i32
    });
    let delay_headroom = request.consider_delay.as_ref().map_or(0, |delay| {
        i64::from(delay_elapsed_tick_bound(delay, delay_tick_bound))
            .saturating_mul(models.saturating_add(4) as i64)
    });
    let baseline_tick = nearest_target_tick(start, request.baseline_target_frame, cache, cancel)?;
    let (first_tick, last_tick) =
        if let Some(observed_frame) = request.drift_search_generation_start_frame {
            let nearest = nearest_generation_start_hits(
                request,
                start,
                observed_frame,
                models,
                delay_headroom,
                cache,
                cancel,
            )?;
            let groups = qualified_generation_hit_ranges(&nearest, request.grace_min_frames);
            let Some((first, last)) = group_tick_bounds(&groups) else {
                return Ok(Vec::new());
            };
            (
                first
                    .saturating_sub(request.drift_range_ticks)
                    .max(start.tick),
                last.saturating_add(request.drift_range_ticks),
            )
        } else {
            let exact =
                exact_generation_start_hits(request, start, models, delay_headroom, cache, cancel)?;
            if exact.is_empty() {
                return Err(TimeleapSearchFailure::GenerationFrameUnreachable);
            }
            let groups = qualified_generation_hit_ranges(&exact, request.grace_min_frames);
            let Some(bounds) = group_tick_bounds(&groups) else {
                return Ok(Vec::new());
            };
            bounds
        };
    collect_confirmed_target_rows(
        request,
        start,
        first_tick,
        last_tick,
        baseline_tick,
        models,
        delay_headroom,
        cache,
        cancel,
    )
}

fn nearest_target_tick(
    start: &ConfirmedTimeleapStart,
    target_frame: i64,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<i64, TimeleapSearchFailure> {
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut status = start.status.clone();
    let mut sfmt =
        cached_sfmt_at(cache, cursor.sfmt_position).map_err(TimeleapSearchFailure::Calculation)?;
    let mut previous = cursor;
    let mut ticks = 0usize;
    while cursor.frame < target_frame && cursor.frame < crate::domain::rng::MAX_SFMT_FRAME {
        if is_cancelled(Some(cancel)) {
            return Err(TimeleapSearchFailure::Cancelled);
        }
        if ticks >= MAX_CANDIDATE_REPLAY_TICKS {
            return Err(TimeleapSearchFailure::Calculation(
                "基準TargetFrame位置の計算が検索上限を超えました。".into(),
            ));
        }
        previous = cursor;
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        ticks += 1;
    }
    Ok(
        if previous.frame.abs_diff(target_frame) < cursor.frame.abs_diff(target_frame) {
            previous.tick
        } else {
            cursor.tick
        },
    )
}

fn exact_generation_start_hits(
    request: &ConfirmedTargetSearchRequest,
    start: &ConfirmedTimeleapStart,
    models: usize,
    delay_headroom: i64,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<Vec<(i64, i64, i64)>, TimeleapSearchFailure> {
    if request.generation_start_frame < start.frame {
        return Ok(Vec::new());
    }
    cache
        .ensure_through(
            request
                .generation_start_frame
                .saturating_add(delay_headroom)
                .saturating_add(311)
                .min(crate::domain::rng::MAX_SFMT_FRAME),
            Some(cancel),
        )
        .map_err(|error| {
            if matches!(error, crate::errors::AppError::Cancelled) {
                TimeleapSearchFailure::Cancelled
            } else {
                TimeleapSearchFailure::Cache(error)
            }
        })?;
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut status = start.status.clone();
    let mut sfmt =
        cached_sfmt_at(cache, cursor.sfmt_position).map_err(TimeleapSearchFailure::Calculation)?;
    if let Some(delay) = request.consider_delay.as_ref() {
        let delay_ticks = i32::try_from(delay.delay_frames.max(0) / 2 + 2).map_err(|_| {
            TimeleapSearchFailure::Calculation("Consider Delayの値を確認してください。".into())
        })?;
        let mut config = request
            .source_config
            .clone()
            .unwrap_or_else(|| TimeleapConfig {
                seed: request.seed,
                start_frame: start.frame,
                target_frame: request.generation_start_frame,
                models,
                min_frame1_elapsed_ticks: 0,
                max_frame1_elapsed_ticks: 0,
                mode: TimeleapMode::Menu,
                is_boy: start.status.is_boy,
                handoff_range_frames: 0,
                courier: delay.courier.clone(),
            });
        config.target_frame = request.generation_start_frame;
        let hits = search_consider_delay_inputs(
            delay,
            delay_ticks,
            &config,
            &status,
            &cursor,
            &sfmt,
            cache,
            &mut HashMap::new(),
            Some(cancel),
            &mut TimeleapSearchStats::default(),
        )
        .map_err(|message| {
            if is_cancelled(Some(cancel)) {
                TimeleapSearchFailure::Cancelled
            } else {
                TimeleapSearchFailure::Calculation(message)
            }
        })?;
        Ok(hits
            .into_iter()
            .map(|hit| {
                (
                    hit.input_cursor.tick,
                    hit.input_cursor.frame,
                    request.generation_start_frame,
                )
            })
            .collect())
    } else {
        let mut ticks = 0usize;
        while cursor.frame < request.generation_start_frame {
            if is_cancelled(Some(cancel)) {
                return Err(TimeleapSearchFailure::Cancelled);
            }
            if ticks >= MAX_CANDIDATE_REPLAY_TICKS {
                return Err(TimeleapSearchFailure::Calculation(
                    "指定生成開始FrameまでのTimeline再生が検索上限を超えました。".into(),
                ));
            }
            advance_one_tick(&mut status, &mut cursor, &mut sfmt);
            ticks += 1;
        }
        Ok((cursor.frame == request.generation_start_frame)
            .then_some((cursor.tick, cursor.frame, request.generation_start_frame))
            .into_iter()
            .collect())
    }
}

fn generation_start_for_target_tick(
    request: &ConfirmedTargetSearchRequest,
    cursor: Cursor,
    status: &TimeleapStatus,
    sfmt: &Sfmt,
    models: usize,
    cancel: &AtomicBool,
) -> Result<Option<i64>, TimeleapSearchFailure> {
    if let Some(delay) = request.consider_delay.as_ref() {
        let mut landing_status = status.clone();
        let mut landing_cursor = cursor;
        let mut landing_sfmt = sfmt.clone();
        consider_delay_landing_frame_cancelable(
            delay,
            &mut landing_status,
            &mut landing_cursor,
            &mut landing_sfmt,
            models,
            crate::domain::rng::MAX_SFMT_FRAME,
            Some(cancel),
        )
        .map_err(TimeleapSearchFailure::Calculation)
    } else {
        Ok(Some(cursor.frame))
    }
}

fn nearest_generation_start_hits(
    request: &ConfirmedTargetSearchRequest,
    start: &ConfirmedTimeleapStart,
    observed_frame: i64,
    models: usize,
    delay_headroom: i64,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<Vec<(i64, i64, i64)>, TimeleapSearchFailure> {
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut status = start.status.clone();
    let mut sfmt =
        cached_sfmt_at(cache, cursor.sfmt_position).map_err(TimeleapSearchFailure::Calculation)?;
    let mut nearest_distance = i64::MAX;
    let mut nearest = Vec::new();
    let mut ticks = 0usize;
    loop {
        if is_cancelled(Some(cancel)) {
            return Err(TimeleapSearchFailure::Cancelled);
        }
        cache
            .ensure_through(
                cursor
                    .frame
                    .saturating_add(delay_headroom)
                    .saturating_add(311)
                    .min(crate::domain::rng::MAX_SFMT_FRAME),
                Some(cancel),
            )
            .map_err(|error| {
                if matches!(error, crate::errors::AppError::Cancelled) {
                    TimeleapSearchFailure::Cancelled
                } else {
                    TimeleapSearchFailure::Cache(error)
                }
            })?;
        if let Some(generation_frame) =
            generation_start_for_target_tick(request, cursor, &status, &sfmt, models, cancel)?
        {
            let distance = generation_frame
                .saturating_sub(observed_frame)
                .saturating_abs();
            match distance.cmp(&nearest_distance) {
                std::cmp::Ordering::Less => {
                    nearest_distance = distance;
                    nearest.clear();
                    nearest.push((cursor.tick, cursor.frame, generation_frame));
                }
                std::cmp::Ordering::Equal => {
                    nearest.push((cursor.tick, cursor.frame, generation_frame));
                }
                std::cmp::Ordering::Greater => {}
            }
        }
        if nearest_distance != i64::MAX
            && cursor.frame > observed_frame.saturating_add(nearest_distance)
        {
            break;
        }
        if cursor.frame >= crate::domain::rng::MAX_SFMT_FRAME {
            break;
        }
        if ticks >= MAX_CANDIDATE_REPLAY_TICKS {
            return Err(TimeleapSearchFailure::Calculation(
                "実測生成開始Frame付近のTimeline検索が上限を超えました。".into(),
            ));
        }
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        ticks += 1;
    }
    Ok(nearest)
}

fn qualified_generation_hit_ranges(
    hits: &[(i64, i64, i64)],
    grace_min_frames: i64,
) -> Vec<(i64, i64)> {
    let mut groups = Vec::new();
    let mut index = 0;
    while index < hits.len() {
        let mut end = index + 1;
        while end < hits.len()
            && hits[end].0 == hits[end - 1].0.saturating_add(1)
            && hits[end].2 == hits[index].2
        {
            end += 1;
        }
        if (end - index).saturating_mul(2) as i64 >= grace_min_frames {
            groups.push((hits[index].0, hits[end - 1].0));
        }
        index = end;
    }
    groups
}

fn group_tick_bounds(groups: &[(i64, i64)]) -> Option<(i64, i64)> {
    Some((
        groups.iter().map(|group| group.0).min()?,
        groups.iter().map(|group| group.1).max()?,
    ))
}

fn collect_confirmed_target_rows(
    request: &ConfirmedTargetSearchRequest,
    start: &ConfirmedTimeleapStart,
    first_tick: i64,
    last_tick: i64,
    baseline_tick: i64,
    models: usize,
    delay_headroom: i64,
    cache: &ObservationCache,
    cancel: &AtomicBool,
) -> Result<Vec<ConfirmedTargetSearchRow>, TimeleapSearchFailure> {
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut status = start.status.clone();
    let mut sfmt =
        cached_sfmt_at(cache, cursor.sfmt_position).map_err(TimeleapSearchFailure::Calculation)?;
    let mut replayed_ticks = 0usize;
    while cursor.tick < first_tick {
        if is_cancelled(Some(cancel)) {
            return Err(TimeleapSearchFailure::Cancelled);
        }
        if replayed_ticks >= MAX_CANDIDATE_REPLAY_TICKS {
            return Err(TimeleapSearchFailure::Calculation(
                "TargetFrame表示範囲までのTimeline再生が検索上限を超えました。".into(),
            ));
        }
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        replayed_ticks += 1;
    }
    let mut rows = Vec::new();
    while cursor.tick <= last_tick && cursor.frame <= crate::domain::rng::MAX_SFMT_FRAME {
        if is_cancelled(Some(cancel)) {
            return Err(TimeleapSearchFailure::Cancelled);
        }
        cache
            .ensure_through(
                cursor
                    .frame
                    .saturating_add(delay_headroom)
                    .saturating_add(311)
                    .min(crate::domain::rng::MAX_SFMT_FRAME),
                Some(cancel),
            )
            .map_err(|error| {
                if matches!(error, crate::errors::AppError::Cancelled) {
                    TimeleapSearchFailure::Cancelled
                } else {
                    TimeleapSearchFailure::Cache(error)
                }
            })?;
        let generation_start_frame =
            generation_start_for_target_tick(request, cursor, &status, &sfmt, models, cancel)?;
        rows.push(ConfirmedTargetSearchRow {
            frame1: request.confirmed_frame1,
            target_frame: cursor.frame,
            drift_frames: confirmed_target_drift_frames(cursor.tick, baseline_tick),
            grace_frames: 0,
            generation_start_frame,
        });
        if cursor.tick == last_tick {
            break;
        }
        if replayed_ticks >= MAX_CANDIDATE_REPLAY_TICKS {
            return Err(TimeleapSearchFailure::Calculation(
                "TargetFrame候補数が検索上限を超えました。".into(),
            ));
        }
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        replayed_ticks += 1;
    }
    assign_confirmed_target_grace_frames(&mut rows);
    Ok(rows)
}
