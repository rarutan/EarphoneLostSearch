use super::super::{
    candidates_have_same_future_state, matching_shift_continuations,
    matching_shift_continuations_cancelable, search_candidates_from_cache,
};
use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn timeleap_event_modes_advance_the_expected_logical_tick_count() {
    for (mode, expected_ticks) in [
        (TimeleapMode::Menu, 0),
        (TimeleapMode::Dialogue, 3),
        (TimeleapMode::Courier, 20),
    ] {
        let mut status = TimeleapStatus::new(1);
        let mut cursor = Cursor {
            frame: 100,
            sfmt_position: 100,
            tick: 50,
        };
        let mut sfmt = Sfmt::new(0x71AE_0001);

        apply_event(
            &mut status,
            &mut cursor,
            &mut sfmt,
            mode,
            &CourierConfig::default(),
        );

        assert_eq!(cursor.tick - 50, expected_ticks, "{mode:?}");
    }
}

#[test]
fn courier_consumption_bitset_matches_fixed_iv_reference() {
    let configs = [
        CourierConfig {
            no_dex: true,
            your_id: false,
            is_egg: false,
            ec: Some(0),
            pid_type: 2,
            pid: 0,
            iv_count: 3,
            fixed_ivs: [-1; 6],
            ability_locked: true,
            ability: 1,
            nature_locked: false,
            nature: 0,
            gender_locked: true,
            gender_ratio: 0,
        },
        CourierConfig {
            no_dex: false,
            your_id: false,
            is_egg: false,
            ec: Some(1),
            pid_type: 3,
            pid: 0,
            iv_count: 2,
            fixed_ivs: [31, -1, -1, -1, 30, -1],
            ability_locked: false,
            ability: 1,
            nature_locked: true,
            nature: 0,
            gender_locked: false,
            gender_ratio: 127,
        },
        CourierConfig {
            no_dex: false,
            your_id: true,
            is_egg: false,
            ec: Some(0),
            pid_type: 0,
            pid: 0,
            iv_count: 0,
            fixed_ivs: [31, -1, 31, -1, 31, -1],
            ability_locked: true,
            ability: 1,
            nature_locked: true,
            nature: 0,
            gender_locked: true,
            gender_ratio: 0,
        },
    ];

    let reference_consumption = |config: &CourierConfig, values: &[u64]| {
        let mut index = 0usize;
        let mut next = || {
            let value = *values.get(index)?;
            index += 1;
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
            let slot = (next()? % 6) as usize;
            if fixed[slot] < 0 {
                fixed[slot] = 31;
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
        Some(index)
    };

    for seed in 0..32 {
        let values = crate::domain::rng::generate_u64_values(seed, 512);
        for config in &configs {
            assert_eq!(
                courier_consumption_from_values(config, &values),
                reference_consumption(config, &values),
                "seed={seed:08x}, config={config:?}"
            );
        }
    }
}

#[test]
fn event_generation_bounds_match_every_complete_start_position() {
    let configs = [
        CourierConfig {
            no_dex: true,
            iv_count: 3,
            ..CourierConfig::default()
        },
        CourierConfig {
            no_dex: true,
            ec: Some(1),
            pid_type: 3,
            iv_count: 2,
            fixed_ivs: [31, -1, -1, -1, 30, -1],
            ability_locked: false,
            nature_locked: true,
            gender_locked: false,
            ..CourierConfig::default()
        },
        CourierConfig {
            no_dex: true,
            ec: Some(0),
            pid_type: 2,
            iv_count: 6,
            fixed_ivs: [31, -1, -1, -1, -1, -1],
            ..CourierConfig::default()
        },
    ];

    for seed in 0..64 {
        let values = crate::domain::rng::generate_u64_values(seed, 1_024);
        for config in &configs {
            let mut expected = CourierConsumptionBounds::default();
            for start in 0..values.len() {
                if let Some(consumed) = courier_consumption_from_values(config, &values[start..]) {
                    expected.min = Some(expected.min.map_or(consumed, |min| min.min(consumed)));
                    expected.max = expected.max.max(consumed);
                }
            }
            assert_eq!(
                courier_consumption_bounds_from_values_cancelable(config, &values, None),
                Ok(expected),
                "seed={seed:08x}, config={config:?}"
            );
        }
    }
}

#[test]
fn event_generation_bounds_keep_duplicate_iv_runs_and_stop_on_first_hit() {
    let config = CourierConfig {
        ec: Some(1),
        pid_type: 3,
        iv_count: 3,
        fixed_ivs: [-1; 6],
        ability_locked: true,
        nature_locked: true,
        gender_locked: true,
        ..CourierConfig::default()
    };

    // The second start sees 2,1,1,2,3 before obtaining its third distinct IV
    // slot. Three later draws fill the remaining IVs, so its complete
    // generation consumes eight outputs; the first start consumes six.
    let duplicate_then_hit = [3_u64, 2, 1, 1, 2, 3, 0, 0, 0, 0];
    assert_eq!(
        courier_consumption_bounds_from_values_cancelable(&config, &duplicate_then_hit, None),
        Ok(CourierConsumptionBounds {
            min: Some(6),
            max: 8,
        })
    );

    // Once 1,2,3 have supplied the three unique slots, trailing duplicates
    // do not extend that start's first-hit draw count.
    let hit_before_duplicate_tail = [1_u64, 2, 3, 3, 3, 0, 0, 0];
    assert_eq!(
        courier_consumption_bounds_from_values_cancelable(
            &config,
            &hit_before_duplicate_tail,
            None,
        ),
        Ok(CourierConsumptionBounds {
            min: Some(6),
            max: 6,
        })
    );
}

#[test]
fn event_generation_bounds_scan_large_sequences_without_per_start_replay() {
    let config = CourierConfig {
        ec: Some(1),
        pid_type: 3,
        iv_count: 3,
        fixed_ivs: [-1; 6],
        ability_locked: true,
        nature_locked: true,
        gender_locked: true,
        ..CourierConfig::default()
    };
    let pattern = [0_u64, 0, 1, 1, 2, 2];
    let values = (0..1_100_000)
        .map(|index| pattern[index % pattern.len()])
        .collect::<Vec<_>>();

    assert_eq!(
        courier_consumption_bounds_from_values_cancelable(&config, &values, None),
        Ok(CourierConsumptionBounds {
            min: Some(7),
            max: 8,
        })
    );
}

#[test]
fn event_delay_tick_without_blink_list_matches_shared_runtime_tick() {
    let remain_values = [0, -1, 1, 30, 36, -5, 0, 2];
    for models in [1, 5, 15] {
        for raining in [false, true] {
            let mut optimized_status = TimeleapStatus::new(models);
            for (index, remain) in optimized_status.model.remain.iter_mut().enumerate() {
                *remain = remain_values[index % remain_values.len()];
            }
            optimized_status.phase = 1;
            optimized_status.fidget_cd = 58;
            optimized_status.is_boy = false;
            optimized_status.raining = raining;
            let mut reference_status = optimized_status.clone();
            let mut optimized_cursor = Cursor {
                frame: 1_000,
                sfmt_position: 1_000,
                tick: 17,
            };
            let mut reference_cursor = optimized_cursor;
            let seed = 0xA110_0000_u32.wrapping_add(models as u32);
            let mut optimized_sfmt = Sfmt::new(seed);
            let mut reference_sfmt = optimized_sfmt.clone();

            for _ in 0..100 {
                advance_one_event_delay_tick(
                    &mut optimized_status,
                    &mut optimized_cursor,
                    &mut optimized_sfmt,
                    models,
                );
                let _ = advance_one_tick_with_fidget_policy::<true, false>(
                    &mut reference_status,
                    &mut reference_cursor,
                    &mut reference_sfmt,
                    models,
                    false,
                );

                assert_eq!(
                    optimized_status, reference_status,
                    "models={models}, rainy={raining}"
                );
                assert_eq!(
                    optimized_cursor, reference_cursor,
                    "models={models}, rainy={raining}"
                );
                let mut optimized_probe = optimized_sfmt.clone();
                let mut reference_probe = reference_sfmt.clone();
                assert_eq!(
                    optimized_probe.next_u64(),
                    reference_probe.next_u64(),
                    "SFMT state differs: models={models}, rainy={raining}"
                );
            }
        }
    }
}

#[test]
fn tick_without_blink_list_preserves_state_and_rng_stream() {
    let remain_values = [0, 1, 2, 30, 36, -1, -5];
    for models in [1, 5, 15] {
        for raining in [false, true] {
            for fidget_cd in [-1, 1, 58] {
                let mut collected_status = TimeleapStatus::new(models);
                for (index, remain) in collected_status.model.remain.iter_mut().enumerate() {
                    *remain = remain_values[index % remain_values.len()];
                }
                collected_status.phase = 1;
                collected_status.fidget_cd = fidget_cd;
                collected_status.is_boy = false;
                collected_status.raining = raining;
                let mut state_only_status = collected_status.clone();
                let mut collected_cursor = Cursor {
                    frame: 1_000,
                    sfmt_position: 1_000,
                    tick: 17,
                };
                let mut state_only_cursor = collected_cursor;
                let mut collected_sfmt = Sfmt::new(0xA110_0000_u32.wrapping_add(models as u32));
                let mut state_only_sfmt = collected_sfmt.clone();

                for _ in 0..100 {
                    let blinks = advance_one_tick(
                        &mut collected_status,
                        &mut collected_cursor,
                        &mut collected_sfmt,
                    );
                    advance_one_tick_without_blink_list(
                        &mut state_only_status,
                        &mut state_only_cursor,
                        &mut state_only_sfmt,
                    );

                    assert_eq!(
                        state_only_status, collected_status,
                        "models={models}, raining={raining}, fidget_cd={fidget_cd}"
                    );
                    assert_eq!(state_only_cursor, collected_cursor);
                    assert_eq!(
                        state_only_sfmt.clone().next_u64(),
                        collected_sfmt.clone().next_u64(),
                        "SFMT stream differs: models={models}, raining={raining}, fidget_cd={fidget_cd}"
                    );
                    assert!(blinks.iter().all(|model| usize::from(*model) < models));
                }
            }
        }
    }
}

#[test]
fn event_consider_delay_from_cached_values_matches_sfmt_path() {
    let seed = 0xDA7A_62F0;
    let mut value_generator = Sfmt::new(seed);
    let values = (0..20_000)
        .map(|_| value_generator.next_u64())
        .collect::<Vec<_>>();
    let delay = ConsiderDelayConfig {
        delay_frames: 62,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            ec: Some(0),
            pid_type: 2,
            iv_count: 3,
            fixed_ivs: [-1; 6],
            ability_locked: true,
            ability: 1,
            nature_locked: false,
            nature: 0,
            gender_locked: true,
            gender_ratio: 0,
            ..CourierConfig::default()
        },
    };
    for (frame, raining) in [(500, false), (507, true), (10_000, false)] {
        let mut normal_status = TimeleapStatus::new(5);
        normal_status
            .model
            .remain
            .copy_from_slice(&[0, 1, 30, -5, 36]);
        normal_status.phase = 1;
        normal_status.raining = raining;
        let mut cached_status = normal_status.clone();
        let mut normal_cursor = Cursor {
            frame,
            sfmt_position: frame,
            tick: 21,
        };
        let mut cached_cursor = normal_cursor;
        let mut normal_sfmt = Sfmt::new(seed);
        for _ in 0..frame {
            normal_sfmt.next_u64();
        }
        let target = frame + 180;
        let normal_reached = apply_consider_delay_cancelable(
            &delay,
            &mut normal_status,
            &mut normal_cursor,
            &mut normal_sfmt,
            5,
            target,
            None,
        )
        .expect("normal Event7 path");
        let cached_reached = apply_event_consider_delay_from_values(
            &delay,
            &mut cached_status,
            &mut cached_cursor,
            5,
            target,
            0,
            &values,
            None,
        )
        .expect("cached-value Event7 path");
        assert_eq!(cached_reached, normal_reached);
        assert_eq!(cached_status, normal_status);
        assert_eq!(cached_cursor, normal_cursor);
        assert_eq!(
            normal_sfmt.next_u64(),
            values[normal_cursor.sfmt_position as usize]
        );
    }
}

#[test]
fn cached_timeleap_shift_prediction_matches_continuous_sfmt_state() {
    let config = TimeleapConfig {
        seed: 0x1234_5678,
        start_frame: 480,
        target_frame: 1_500,
        models: 1,
        min_frame1_elapsed_ticks: 30,
        max_frame1_elapsed_ticks: 90,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 20,
        courier: CourierConfig::default(),
    };
    let cache = crate::domain::rng::generate_observation_cache(
        config.seed,
        0..=config.target_frame,
        config.target_frame,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cached SFMT stream");
    let candidate = search_candidates_from_cache(&config, &cache)
        .expect("search from cached SFMT stream")
        .into_iter()
        .flat_map(|group| group.candidates)
        .next()
        .expect("timeleap candidate");

    let mut continuous_sfmt = Sfmt::new(config.seed);
    for _ in 0..candidate.post_sfmt_position {
        continuous_sfmt.next_u64();
    }
    let mut status = candidate.post_status.clone();
    let mut cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: candidate.post_tick,
    };
    let mut expected = Vec::new();
    for _ in 0..2_000 {
        let blinks = advance_one_tick(&mut status, &mut cursor, &mut continuous_sfmt);
        if blinks.contains(&0) {
            expected.push(cursor.tick.saturating_mul(2));
        }
    }

    let cached_prediction =
        predict_shift_times_until(&cache, &candidate, 0, cursor.tick.saturating_mul(2));
    assert_eq!(cached_prediction, expected);
}

#[test]
fn matching_shift_continuations_resume_at_each_matching_last_blink() {
    let config = TimeleapConfig {
        seed: 0x1234_5678,
        start_frame: 480,
        target_frame: 1_500,
        models: 1,
        min_frame1_elapsed_ticks: 30,
        max_frame1_elapsed_ticks: 90,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 20,
        courier: CourierConfig::default(),
    };
    let cache = crate::domain::rng::generate_observation_cache(
        config.seed,
        0..=20_000,
        20_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate test cache");
    let candidate = search_candidates_from_cache(&config, &cache)
        .expect("search test candidates")
        .into_iter()
        .flat_map(|group| group.candidates)
        .next()
        .expect("at least one candidate");
    let cancelled = AtomicBool::new(true);
    assert!(matching_shift_continuations_cancelable(
        &cache,
        &candidate,
        0,
        &[10_000],
        0,
        0,
        20_000,
        Some(&cancelled),
    )
    .is_none());
    let prediction_limit = candidate.post_tick.saturating_mul(2) + 10_000;
    let predicted = predict_shift_times_until(&cache, &candidate, 0, prediction_limit);
    let observed = *predicted.get(2).expect("three predicted blinks");

    let matches = matching_shift_continuations(&cache, &candidate, 0, &[observed], 0, 0, observed);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].post_tick.saturating_mul(2), observed);
    assert!(matches[0].post_tick > candidate.post_tick);

    let ambiguous =
        matching_shift_continuations(&cache, &candidate, 0, &[observed], 0, 200, observed + 200);
    assert!(
        ambiguous.len() > 1,
        "tolerance can retain multiple positions"
    );
    assert!(ambiguous
        .windows(2)
        .any(|pair| !candidates_have_same_future_state(&pair[0], &pair[1])));
}
