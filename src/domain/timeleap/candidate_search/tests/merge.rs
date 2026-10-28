use super::*;

#[test]
fn generated_reachable_targets_survive_frame1_merging_and_parallel_search() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    struct Sample {
        name: &'static str,
        seed: u32,
        models: usize,
        mode: TimeleapMode,
        is_boy: bool,
        delay: ConsiderDelayConfig,
        frame1_tick: i64,
        a_wait_ticks: usize,
    }

    let random_courier = CourierConfig::default();
    let samples = [
        Sample {
            name: "menu / one model / zero configured delay",
            seed: 0x1020_3040,
            models: 1,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay: ConsiderDelayConfig {
                delay_frames: 0,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::Normal),
                courier: random_courier.clone(),
            },
            frame1_tick: 0,
            a_wait_ticks: 0,
        },
        Sample {
            name: "menu / five models / common legendary",
            seed: 0x2233_4455,
            models: 5,
            mode: TimeleapMode::Menu,
            is_boy: false,
            delay: ConsiderDelayConfig {
                delay_frames: 18,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::CommonLegendary),
                courier: random_courier.clone(),
            },
            frame1_tick: 7,
            a_wait_ticks: 4,
        },
        Sample {
            name: "dialogue / seventeen models / first Kartana",
            seed: 0x3344_5566,
            models: 17,
            mode: TimeleapMode::Dialogue,
            is_boy: true,
            delay: ConsiderDelayConfig {
                delay_frames: 0,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
                courier: random_courier.clone(),
            },
            frame1_tick: 11,
            a_wait_ticks: 6,
        },
        Sample {
            name: "menu / twenty-two models / first Kartana / maximum profile input",
            seed: 0x4455_6677,
            models: 22,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay: ConsiderDelayConfig {
                delay_frames: 9_998,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
                courier: random_courier.clone(),
            },
            frame1_tick: 3,
            a_wait_ticks: 2,
        },
        Sample {
            name: "menu / fifteen models / first Xurkitree",
            seed: 0x5566_7788,
            models: 15,
            mode: TimeleapMode::Menu,
            is_boy: false,
            delay: ConsiderDelayConfig {
                delay_frames: 98,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstXurkitree),
                courier: random_courier.clone(),
            },
            frame1_tick: 17,
            a_wait_ticks: 9,
        },
        Sample {
            name: "menu / twenty-two models / wide Frame1 window before A",
            seed: 0x5a17_c0de,
            models: 22,
            mode: TimeleapMode::Menu,
            is_boy: false,
            delay: ConsiderDelayConfig {
                delay_frames: 18,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::CommonLegendary),
                courier: random_courier.clone(),
            },
            frame1_tick: 120,
            a_wait_ticks: 400,
        },
        Sample {
            name: "courier / No Dex / random event generation",
            seed: 0x6677_8899,
            models: 5,
            mode: TimeleapMode::Courier,
            is_boy: true,
            delay: ConsiderDelayConfig {
                delay_frames: 24,
                generator: ConsiderDelayGenerator::Event,
                courier: CourierConfig {
                    no_dex: true,
                    ..CourierConfig::default()
                },
            },
            frame1_tick: 5,
            a_wait_ticks: 3,
        },
        Sample {
            name: "courier / Your ID / event generation during delay",
            seed: 0x7788_99aa,
            models: 15,
            mode: TimeleapMode::Courier,
            is_boy: false,
            delay: ConsiderDelayConfig {
                delay_frames: 10,
                generator: ConsiderDelayGenerator::Event,
                courier: CourierConfig {
                    your_id: true,
                    ..CourierConfig::default()
                },
            },
            frame1_tick: 8,
            a_wait_ticks: 5,
        },
        Sample {
            name: "courier / egg / no in-delay event generation",
            seed: 0x8899_aabb,
            models: 4,
            mode: TimeleapMode::Courier,
            is_boy: true,
            delay: ConsiderDelayConfig {
                delay_frames: 98,
                generator: ConsiderDelayGenerator::Event,
                courier: CourierConfig {
                    your_id: true,
                    is_egg: true,
                    ..CourierConfig::default()
                },
            },
            frame1_tick: 13,
            a_wait_ticks: 7,
        },
    ];

    let start_frame = 480;
    let mut saw_frame1_state_merge = false;

    for sample in samples {
        // Build a known-realizable sample through the same Timeline event
        // and Consider Delay functions the production search is meant to
        // invert. Search must recover this exact Frame1/Target pair.
        let mut known_status = TimeleapStatus::new(sample.models);
        known_status.is_boy = sample.is_boy;
        let mut known_cursor = Cursor {
            frame: start_frame,
            sfmt_position: start_frame,
            tick: 0,
        };
        let mut known_sfmt = Sfmt::new(sample.seed);
        for _ in 0..start_frame {
            known_sfmt.next_u64();
        }
        for _ in 0..sample.frame1_tick {
            advance_one_tick(&mut known_status, &mut known_cursor, &mut known_sfmt);
        }
        let known_frame1 = known_cursor.frame;
        apply_event(
            &mut known_status,
            &mut known_cursor,
            &mut known_sfmt,
            sample.mode,
            &sample.delay.courier,
        );
        for _ in 0..sample.a_wait_ticks {
            advance_one_tick(&mut known_status, &mut known_cursor, &mut known_sfmt);
        }
        let known_a_input = known_cursor.frame;
        let _ = apply_consider_delay(
            &sample.delay,
            &mut known_status,
            &mut known_cursor,
            &mut known_sfmt,
            sample.models,
            i64::MAX / 2,
        )
        .unwrap_or_else(|error| panic!("{} generated delay: {error}", sample.name));
        let target_frame = known_cursor.frame;
        assert!(
            target_frame > known_a_input,
            "{} must include delay",
            sample.name
        );
        let cache_end = target_frame.saturating_add(10_000);
        let cache = crate::domain::rng::generate_observation_cache(
            sample.seed,
            0..=cache_end,
            cache_end,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap_or_else(|error| panic!("{} cache: {error}", sample.name));

        let frame1_min = 0;
        let frame1_max = sample.frame1_tick.saturating_add(80);
        let config = TimeleapConfig {
            seed: sample.seed,
            start_frame,
            target_frame,
            models: sample.models,
            min_frame1_elapsed_ticks: frame1_min,
            max_frame1_elapsed_ticks: frame1_max,
            mode: sample.mode,
            is_boy: sample.is_boy,
            handoff_range_frames: 20,
            courier: sample.delay.courier.clone(),
        };
        let timeline_start = TimeleapTimelineStart::initial(start_frame, sample.models);

        let (serial, _) = search_candidates_from_timeline_with_stats(
            &config,
            &timeline_start,
            0,
            &cache,
            None,
            true,
            Some(&sample.delay),
            false,
        )
        .unwrap_or_else(|error| panic!("{} serial search: {error}", sample.name));
        let (optimized, stats) = search_candidates_from_timeline_with_stats(
            &config,
            &timeline_start,
            0,
            &cache,
            None,
            true,
            Some(&sample.delay),
            true,
        )
        .unwrap_or_else(|error| panic!("{} optimized search: {error}", sample.name));

        let group_summary = |groups: &[TimeleapCandidateGroup]| {
            groups
                .iter()
                .map(|group| {
                    (
                        group.frame1_tick_start,
                        group.frame1_tick_end,
                        group.grace_frames,
                        group.target_frame_start,
                        group.target_frame_end,
                        group.target_grace_frames,
                        group.candidates.len(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            group_summary(&optimized),
            group_summary(&serial),
            "{} optimized output; optimized candidates={:?}; independent candidates={:?}",
            sample.name,
            optimized
                .iter()
                .flat_map(|group| &group.candidates)
                .map(|candidate| (
                    candidate.frame1,
                    candidate.frame1_tick,
                    candidate
                        .target_inputs
                        .iter()
                        .map(|input| input.frame)
                        .collect::<Vec<_>>(),
                ))
                .collect::<Vec<_>>(),
            serial
                .iter()
                .flat_map(|group| &group.candidates)
                .map(|candidate| (
                    candidate.frame1,
                    candidate.frame1_tick,
                    candidate
                        .target_inputs
                        .iter()
                        .map(|input| input.frame)
                        .collect::<Vec<_>>(),
                ))
                .collect::<Vec<_>>(),
        );
        if sample.name == "menu / twenty-two models / wide Frame1 window before A" {
            assert!(
                    optimized.iter().any(|group| {
                        group.frame1_tick_start <= frame1_max
                            && group.frame1_tick_end > frame1_max
                    }),
                    "the Consider Delay regression must retain the whole run past the seconds upper bound"
                );
        }
        let first_state_mismatch = optimized
            .iter()
            .flat_map(|group| &group.candidates)
            .zip(serial.iter().flat_map(|group| &group.candidates))
            .find(|(optimized, serial)| optimized != serial);
        assert!(
                first_state_mismatch.is_none(),
                "{} candidate states differ: {:?}",
                sample.name,
                first_state_mismatch.map(|(optimized, serial)| {
                    format!(
                        "tick={}, frame_equal={}, pre_status_equal={}, target_cursor=({},{},{})/({},{},{}), post_status_equal={}, model_equal={}, post_remain={:?}/{:?}, phase_equal={}, fidget_cd={}/{}, target_inputs_equal={}",
                        optimized.frame1_tick,
                        optimized.frame1 == serial.frame1,
                        optimized.pre_status == serial.pre_status,
                        optimized.post_tick,
                        optimized.post_frame,
                        optimized.post_sfmt_position,
                        serial.post_tick,
                        serial.post_frame,
                        serial.post_sfmt_position,
                        optimized.post_status == serial.post_status,
                        optimized.post_status.model.remain == serial.post_status.model.remain,
                        optimized.post_status.model.remain,
                        serial.post_status.model.remain,
                        optimized.post_status.phase == serial.post_status.phase,
                        optimized.post_status.fidget_cd,
                        serial.post_status.fidget_cd,
                        optimized.target_inputs == serial.target_inputs,
                    )
                })
            );
        assert!(
                optimized.iter().flat_map(|group| &group.candidates).any(|candidate| {
                    candidate.frame1_tick == sample.frame1_tick
                        && candidate.frame1 == known_frame1
                        && candidate.post_frame == target_frame
                }),
                "{} lost a constructed reachable result: Frame1={known_frame1} (tick {}), A={known_a_input}, Target={target_frame}; candidates={:?}",
                sample.name,
                sample.frame1_tick,
                optimized
                    .iter()
                    .flat_map(|group| &group.candidates)
                    .map(|candidate| (candidate.frame1, candidate.frame1_tick, candidate.post_frame))
                    .collect::<Vec<_>>()
            );
        assert!(
            stats.merged_timeline_work_items <= stats.candidates_tested,
            "{} cannot create more merged Timeline work than Frame1 inputs",
            sample.name
        );
        saw_frame1_state_merge |=
            stats.merged_timeline_work_items < stats.candidates_tested && stats.merge_hits > 0;
    }

    assert!(
        saw_frame1_state_merge,
        "the test matrix must exercise pre-A-input Timeline merging"
    );
}

#[test]
fn generated_timeleap_merging_matches_independent_target_checks() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    let modes = [
        TimeleapMode::Menu,
        TimeleapMode::Dialogue,
        TimeleapMode::Courier,
    ];
    let delay_types = [
        DelayType::Normal,
        DelayType::CommonLegendary,
        DelayType::FirstKartana,
        DelayType::FirstXurkitree,
    ];
    let model_counts = [1, 2, 5, 13, 22];
    let start_frame = 480;
    let mut saw_reachable_target = false;
    let mut saw_unreachable_target = false;
    let mut simulated_targets = 0usize;
    let mut reachable_target_count = 0usize;
    let mut unreachable_target_count = 0usize;
    let mut reachable_example = None;
    let mut unreachable_example = None;

    // Deterministic pseudo-random coverage: every sample is first generated
    // by the real event + delay transitions, then searched once per-origin
    // and once with the production Timeline-sharing path.
    for sample_index in 0..24u32 {
        let seed = 0x91e1_0da5u32
            .wrapping_add(sample_index.wrapping_mul(0x9e37_79b9))
            .rotate_left(sample_index % 31);
        let models = model_counts[sample_index as usize % model_counts.len()];
        let mode = modes[(sample_index as usize / 2) % modes.len()];
        let delay_type = delay_types[(sample_index as usize / 3) % delay_types.len()];
        let frame1_tick = 1 + (sample_index as i64 % 11);
        let a_wait_ticks = (sample_index as usize * 7) % 9;
        let courier = CourierConfig {
            no_dex: sample_index % 3 == 0,
            your_id: sample_index % 4 == 0,
            is_egg: sample_index % 8 == 0,
            ..CourierConfig::default()
        };
        let delay = ConsiderDelayConfig {
            delay_frames: [0, 18, 98, 312][sample_index as usize % 4],
            generator: if mode == TimeleapMode::Courier {
                ConsiderDelayGenerator::Event
            } else {
                ConsiderDelayGenerator::FixedSymbol(delay_type)
            },
            courier: courier.clone(),
        };

        let mut actual_status = TimeleapStatus::new(models);
        actual_status.is_boy = sample_index % 2 == 0;
        let mut actual_cursor = Cursor {
            frame: start_frame,
            sfmt_position: start_frame,
            tick: 0,
        };
        let mut actual_sfmt = Sfmt::new(seed);
        for _ in 0..start_frame {
            actual_sfmt.next_u64();
        }
        for _ in 0..frame1_tick {
            advance_one_tick(&mut actual_status, &mut actual_cursor, &mut actual_sfmt);
        }
        apply_event(
            &mut actual_status,
            &mut actual_cursor,
            &mut actual_sfmt,
            mode,
            &courier,
        );
        for _ in 0..a_wait_ticks {
            advance_one_tick(&mut actual_status, &mut actual_cursor, &mut actual_sfmt);
        }
        let _ = apply_consider_delay(
            &delay,
            &mut actual_status,
            &mut actual_cursor,
            &mut actual_sfmt,
            models,
            i64::MAX / 2,
        )
        .unwrap_or_else(|error| panic!("sample {sample_index} delay: {error}"));
        let planted_target = actual_cursor.frame;
        let cache_end = planted_target.saturating_add(10_000);
        let cache = crate::domain::rng::generate_observation_cache(
            seed,
            0..=cache_end,
            cache_end,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap_or_else(|error| panic!("sample {sample_index} cache: {error}"));
        let timeline_start = TimeleapTimelineStart::initial(start_frame, models);

        for target_frame in [
            planted_target.saturating_sub(1),
            planted_target,
            planted_target.saturating_add(1),
        ] {
            let config = TimeleapConfig {
                seed,
                start_frame,
                target_frame,
                models,
                min_frame1_elapsed_ticks: 0,
                max_frame1_elapsed_ticks: frame1_tick.saturating_add(12),
                mode,
                is_boy: sample_index % 2 == 0,
                handoff_range_frames: 20,
                courier: courier.clone(),
            };

            // No candidate Timeline is grouped and no completed A-scan is
            // reused: this is the per-origin baseline for target reachability.
            let (independent, _) = search_candidates_from_timeline_with_stats(
                &config,
                &timeline_start,
                0,
                &cache,
                None,
                true,
                Some(&delay),
                false,
            )
            .unwrap_or_else(|error| {
                panic!("sample {sample_index}, target {target_frame} independent search: {error}")
            });
            let (shared, _) = search_candidates_from_timeline_with_stats(
                &config,
                &timeline_start,
                0,
                &cache,
                None,
                true,
                Some(&delay),
                true,
            )
            .unwrap_or_else(|error| {
                panic!("sample {sample_index}, target {target_frame} shared search: {error}")
            });

            assert_eq!(
                    shared, independent,
                    "sample {sample_index}: shared and independent Target results differ; seed={seed:08X}, models={models}, mode={mode:?}, delay={delay:?}, target={target_frame}"
                );
            saw_reachable_target |= !shared.is_empty();
            saw_unreachable_target |= shared.is_empty();
            if shared.is_empty() {
                unreachable_target_count += 1;
                unreachable_example.get_or_insert_with(|| {
                        format!(
                            "seed={seed:08X}, models={models}, mode={mode:?}, delay={delay:?}, target={target_frame}"
                        )
                    });
            } else {
                reachable_target_count += 1;
                reachable_example.get_or_insert_with(|| {
                        format!(
                            "seed={seed:08X}, models={models}, mode={mode:?}, delay={delay:?}, target={target_frame}"
                        )
                    });
            }
            if target_frame == planted_target {
                assert!(
                        shared.iter().flat_map(|group| &group.candidates).any(|candidate| {
                            candidate.frame1_tick == frame1_tick
                                && candidate.post_frame == planted_target
                        }),
                        "sample {sample_index}: planted reachable target disappeared; seed={seed:08X}, models={models}, mode={mode:?}, delay={delay:?}"
                    );
            }
            simulated_targets += 1;
        }
    }

    assert_eq!(simulated_targets, 72);
    assert!(
        saw_reachable_target && saw_unreachable_target,
        "the generated sweep must exercise both reachable and unreachable Targets"
    );
    eprintln!(
            "24 generated Timeleap scenarios / {simulated_targets} targets: reachable={reachable_target_count}, unreachable={unreachable_target_count}; example reachable: {}; example unreachable: {}",
            reachable_example.as_deref().unwrap_or("none"),
            unreachable_example.as_deref().unwrap_or("none"),
        );
}

#[test]
fn equivalent_candidates_use_all_future_state_fields() {
    let status = TimeleapStatus::new(1);
    let candidate = |position, frame, tick| TimeleapCandidate {
        frame1: 1,
        frame1_tick: 1,
        post_tick: tick,
        grace_frames: 0,
        pre_frame: 0,
        pre_sfmt_position: 0,
        pre_status: status.clone(),
        post_frame: frame,
        post_sfmt_position: position,
        post_status: status.clone(),
        target_inputs: Vec::new().into(),
    };
    assert!(candidates_have_same_future_state(
        &candidate(4, 4, 2),
        &candidate(4, 4, 2)
    ));
    assert!(!candidates_have_same_future_state(
        &candidate(4, 4, 2),
        &candidate(5, 4, 2)
    ));
    assert!(!candidates_have_same_future_state(
        &candidate(4, 4, 2),
        &candidate(4, 6, 3)
    ));
}

#[test]
fn target_table_merge_key_distinguishes_each_npc_model_state() {
    let cursor = Cursor {
        frame: 1_000,
        sfmt_position: 1_000,
        tick: 500,
    };
    let baseline = TimeleapStatus::new(3);
    let baseline_key = future_state_key(cursor, &baseline);
    let mut known_states = HashMap::new();
    known_states.insert(baseline_key.clone(), 0);

    for model_index in 0..baseline.model.remain.len() {
        let mut different_npc_state = baseline.clone();
        different_npc_state.model.remain[model_index] = -5;
        let key = future_state_key(cursor, &different_npc_state);

        assert_ne!(
            key, baseline_key,
            "NPC slot {model_index} is part of identity"
        );
        assert!(
            !known_states.contains_key(&key),
            "a different NPC slot state must not merge into the Target table work item"
        );
    }

    assert_eq!(
        known_states.get(&future_state_key(cursor, &baseline)),
        Some(&0),
        "an exactly equal ordered state remains reusable"
    );
}

#[test]
fn target_table_merge_key_treats_remain_zero_and_one_as_equivalent() {
    let cursor = Cursor {
        frame: 1_000,
        sfmt_position: 1_000,
        tick: 500,
    };
    let mut zero_status = TimeleapStatus::new(4);
    zero_status.model.remain = vec![0, 1, 2, -5];
    let mut one_status = zero_status.clone();
    one_status.model.remain = vec![1, 0, 2, -5];

    let zero_key = future_state_key(cursor, &zero_status);
    let one_key = future_state_key(cursor, &one_status);
    let mut shared_states = HashMap::new();
    shared_states.insert(zero_key.clone(), 7);

    assert_eq!(zero_key, one_key);
    assert_eq!(shared_states.get(&one_key), Some(&7));
    assert_eq!(zero_status.model.remain, vec![0, 1, 2, -5]);
    assert_eq!(one_status.model.remain, vec![1, 0, 2, -5]);

    let mut different_cooldown = one_status.clone();
    different_cooldown.model.remain[2] = 3;
    assert_ne!(
        future_state_key(cursor, &different_cooldown),
        zero_key,
        "non-idle remain values remain part of state identity"
    );

    let reordered_non_idle = TimeleapStatus {
        model: crate::domain::rng::ModelStatus {
            remain: vec![2, 0, 1, -5],
        },
        ..zero_status.clone()
    };
    assert_ne!(
        future_state_key(cursor, &reordered_non_idle),
        zero_key,
        "normalizing 0/1 must not reorder NPC slots or merge distinct model states"
    );

    let different_sfmt_cursor = Cursor {
        sfmt_position: cursor.sfmt_position + 1,
        ..cursor
    };
    assert_ne!(
        future_state_key(different_sfmt_cursor, &one_status),
        zero_key,
        "SFMT position remains part of state identity"
    );
}

#[test]
fn target_table_merge_key_keeps_shifted_npc_remain_slots_distinct() {
    let cursor = Cursor {
        frame: 502,
        sfmt_position: 502,
        tick: 251,
    };
    let mut first_status = TimeleapStatus::new(3);
    first_status.model.remain = vec![0, 2, 0];
    let mut shifted_status = TimeleapStatus::new(3);
    shifted_status.model.remain = vec![2, 0, 0];

    let first_key = future_state_key(cursor, &first_status);
    let shifted_key = future_state_key(cursor, &shifted_status);
    let mut states = HashMap::new();
    states.insert(first_key.clone(), "500 / 0,2,0");
    states.insert(shifted_key.clone(), "501 / 2,0,0");

    assert_ne!(first_key, shifted_key);
    assert_eq!(states.len(), 2);
    assert_eq!(states.get(&first_key), Some(&"500 / 0,2,0"));
    assert_eq!(states.get(&shifted_key), Some(&"501 / 2,0,0"));
}

#[test]
fn shared_target_timeline_restores_each_frame1_origin_and_npc_state() {
    let mut first_status = TimeleapStatus::new(3);
    first_status.model.remain = vec![-5, 0, 1];
    let mut second_status = TimeleapStatus::new(3);
    second_status.model.remain = vec![0, -5, 1];
    let origins = vec![
        Frame1CandidateOrigin {
            frame1: 500,
            frame1_tick: 10,
            pre_frame: 500,
            pre_sfmt_position: 510,
            pre_status: first_status.clone(),
            scan_start_tick: 30,
        },
        Frame1CandidateOrigin {
            frame1: 505,
            frame1_tick: 12,
            pre_frame: 505,
            pre_sfmt_position: 517,
            pre_status: second_status.clone(),
            scan_start_tick: 34,
        },
    ];
    let mut result_status = TimeleapStatus::new(3);
    result_status.model.remain = vec![1, 0, -5];
    let result_cursor = Cursor {
        frame: 900,
        sfmt_position: 930,
        tick: 50,
    };

    let input_cursor = Cursor {
        frame: 880,
        sfmt_position: 900,
        tick: 48,
    };
    let candidates = restore_shared_timeline_result(
        origins,
        30,
        vec![ConsiderDelayHit {
            input_cursor,
            landing_cursor: result_cursor,
            landing_status: result_status.clone(),
        }],
    );

    assert_eq!(candidates.len(), 2);
    assert_eq!(
        (
            candidates[0].frame1,
            candidates[0].frame1_tick,
            candidates[0].pre_frame,
            candidates[0].pre_sfmt_position,
            candidates[0].pre_status.clone(),
            candidates[0].post_tick,
        ),
        (500, 10, 500, 510, first_status, 50)
    );
    assert_eq!(
        (
            candidates[1].frame1,
            candidates[1].frame1_tick,
            candidates[1].pre_frame,
            candidates[1].pre_sfmt_position,
            candidates[1].pre_status.clone(),
            candidates[1].post_tick,
        ),
        (505, 12, 505, 517, second_status, 54)
    );
    assert!(candidates.iter().all(|candidate| {
        candidate.post_frame == result_cursor.frame
            && candidate.post_sfmt_position == result_cursor.sfmt_position
            && candidate.post_status == result_status
    }));
}

#[test]
fn surviving_selection_prefers_set_or_later_middle() {
    let status = TimeleapStatus::new(1);
    let a = TimeleapCandidate {
        frame1: 10,
        frame1_tick: 0,
        post_tick: 0,
        grace_frames: 0,
        pre_frame: 0,
        pre_sfmt_position: 0,
        pre_status: status.clone(),
        post_frame: 0,
        post_sfmt_position: 1,
        post_status: status.clone(),
        target_inputs: Vec::new().into(),
    };
    let b = TimeleapCandidate {
        frame1: 12,
        frame1_tick: 0,
        post_tick: 0,
        grace_frames: 0,
        pre_frame: 0,
        pre_sfmt_position: 0,
        pre_status: status.clone(),
        post_frame: 0,
        post_sfmt_position: 2,
        post_status: status.clone(),
        target_inputs: Vec::new().into(),
    };
    let c = TimeleapCandidate {
        frame1: 14,
        frame1_tick: 0,
        post_tick: 0,
        grace_frames: 0,
        pre_frame: 0,
        pre_sfmt_position: 0,
        pre_status: status.clone(),
        post_frame: 0,
        post_sfmt_position: 3,
        post_status: status,
        target_inputs: Vec::new().into(),
    };
    assert_eq!(select_surviving_frame1(&[&a, &b, &c], Some(12)), Some(12));
    assert_eq!(select_surviving_frame1(&[&a, &b, &c], None), Some(12));
}
