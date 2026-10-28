use super::*;
use crate::domain::rng::{
    advance_timeline, encounter_plan_with_observed_rotom_and_initial_load,
    encounter_plan_with_pre_rotom_consumption, encounter_plan_with_rotom, production_timeline,
    ModelStatus, ObservationMatch, Sfmt,
};
use crate::domain::timing::{blink_ticks_to_seconds, game_frames_to_seconds, DEFAULT_FPS};
use std::collections::HashMap;
use std::time::Duration;

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

fn main_reference_advance(values: &[u64], start: i64, models: usize, ticks: i64) -> Option<i64> {
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

mod configuration;
mod correction;
mod guidance;
mod production;
mod timer;
