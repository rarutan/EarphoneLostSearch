use super::*;

#[test]
fn idle_tick_batch_matches_exact_tick_transitions() {
    let mut batched_status = TimeleapStatus::new(3);
    batched_status.model.remain.copy_from_slice(&[3, 4, 35]);
    batched_status.fidget_cd = 7;
    let mut stepped_status = batched_status.clone();
    let mut batched_cursor = Cursor {
        frame: 500,
        sfmt_position: 500,
        tick: 20,
    };
    let mut stepped_cursor = batched_cursor;
    let mut batched_sfmt = Sfmt::new(0x1D1E_7A11);
    for _ in 0..500 {
        batched_sfmt.next_u64();
    }
    let mut stepped_sfmt = batched_sfmt.clone();

    let idle_ticks = idle_tick_run(&batched_status, 100);
    assert_eq!(idle_ticks, 2);
    advance_idle_ticks(&mut batched_status, &mut batched_cursor, idle_ticks);
    for _ in 0..idle_ticks {
        advance_one_tick_without_blink_list(
            &mut stepped_status,
            &mut stepped_cursor,
            &mut stepped_sfmt,
        );
    }
    assert_eq!(batched_status, stepped_status);
    assert_eq!(batched_cursor, stepped_cursor);
    assert_eq!(batched_sfmt.next_u64(), stepped_sfmt.next_u64());

    advance_one_tick_without_blink_list(
        &mut batched_status,
        &mut batched_cursor,
        &mut batched_sfmt,
    );
    advance_one_tick_without_blink_list(
        &mut stepped_status,
        &mut stepped_cursor,
        &mut stepped_sfmt,
    );
    assert_eq!(batched_status, stepped_status);
    assert_eq!(batched_cursor, stepped_cursor);
    assert_eq!(batched_sfmt.next_u64(), stepped_sfmt.next_u64());

    batched_status.raining = true;
    assert_eq!(idle_tick_run(&batched_status, 100), 0);
    batched_status.raining = false;
    batched_status.model.remain[0] = -1;
    assert_eq!(idle_tick_run(&batched_status, 100), 0);
}

#[test]
fn no_blink_run_from_cached_values_matches_each_tick_transition() {
    let seed = 0x5AFE_F12E;
    let values = crate::domain::rng::generate_u64_values(seed, 2_000);
    for (fidget_cd, remain) in [
        (-1, vec![0, 1, 0, 1, 0]),
        (0, vec![0, 2, 1, 4, 0]),
        (17, vec![1, 30, 0, 35, 1]),
        (-1, vec![-5, -2, 0, 1, 2]),
        (3, vec![-4, -2, 0, 2, 5]),
        (0, vec![-5, -4, -3, 3, 5]),
    ] {
        let mut fast_status = TimeleapStatus::new(5);
        fast_status.model.remain = remain;
        fast_status.fidget_cd = fidget_cd;
        let mut reference_status = fast_status.clone();
        let mut fast_cursor = Cursor {
            frame: 500,
            sfmt_position: 500,
            tick: 9,
        };
        let mut reference_cursor = fast_cursor;
        let mut reference_sfmt = Sfmt::new(seed);
        for _ in 0..500 {
            reference_sfmt.next_u64();
        }

        while fast_cursor.frame < 800 {
            let skipped = advance_no_blink_run_from_values(
                &mut fast_status,
                &mut fast_cursor,
                800,
                1_000,
                0,
                &values,
                true,
            );
            if skipped == 0 {
                assert!(advance_one_tick_state_only_from_values(
                    &mut fast_status,
                    &mut fast_cursor,
                    5,
                    true,
                    0,
                    &values,
                ));
            }
            advance_one_tick_without_blink_list(
                &mut reference_status,
                &mut reference_cursor,
                &mut reference_sfmt,
            );
            while reference_cursor.tick < fast_cursor.tick {
                advance_one_tick_without_blink_list(
                    &mut reference_status,
                    &mut reference_cursor,
                    &mut reference_sfmt,
                );
            }
            assert_eq!(fast_status, reference_status, "fidget_cd={fidget_cd}");
            assert_eq!(fast_cursor, reference_cursor, "fidget_cd={fidget_cd}");
            let mut expected_sfmt = reference_sfmt.clone();
            assert_eq!(
                values[fast_cursor.sfmt_position as usize],
                expected_sfmt.next_u64()
            );
        }
    }

    let mut rainy = TimeleapStatus::new(2);
    rainy.model.remain = vec![0, 1];
    rainy.raining = true;
    assert_eq!(
        advance_no_blink_run_from_values(
            &mut rainy,
            &mut Cursor {
                frame: 100,
                sfmt_position: 100,
                tick: 0,
            },
            500,
            100,
            0,
            &values,
            true,
        ),
        0
    );
}

#[test]
fn indexed_no_blink_run_matches_linear_trigger_scan() {
    let mut values = vec![1_u64; 1_024];
    for index in [0, 7, 31, 127, 255, 511, 1_023] {
        values[index] = 0;
    }
    let trigger_index = NoBlinkTriggerIndex::new(&values);

    for (offset, fidget_cd, remain) in [
        (0, -1, vec![0, 1, 0, 1, 0]),
        (1, 12, vec![0, 1, 0, 1, 0]),
        (8, 0, vec![0, 2, 1, 4, 0]),
        (32, 17, vec![-5, -2, 0, 1, 2]),
        (128, 3, vec![-4, -2, 0, 2, 5]),
        (512, -1, vec![1, 1, 1, 1, 1]),
    ] {
        let mut linear_status = TimeleapStatus::new(remain.len());
        linear_status.model.remain.clone_from(&remain);
        linear_status.fidget_cd = fidget_cd;
        let mut indexed_status = linear_status.clone();
        let mut linear_cursor = Cursor {
            frame: 500,
            sfmt_position: offset,
            tick: 9,
        };
        let mut indexed_cursor = linear_cursor;
        let linear_skipped = advance_no_blink_run_from_values(
            &mut linear_status,
            &mut linear_cursor,
            800,
            1_000,
            0,
            &values,
            true,
        );
        let indexed_skipped = advance_no_blink_run_from_values_with_index(
            &mut indexed_status,
            &mut indexed_cursor,
            800,
            1_000,
            0,
            &values,
            true,
            Some(&trigger_index),
        );

        assert_eq!(indexed_skipped, linear_skipped);
        assert_eq!(indexed_status, linear_status);
        assert_eq!(indexed_cursor, linear_cursor);
    }
}

fn shared_scan_test_key(frame: i64) -> ConsiderDelayScanKey {
    let status = TimeleapStatus::new(1);
    ConsiderDelayScanKey {
        window_start: 1_000,
        state: future_state_key(
            Cursor {
                frame,
                sfmt_position: frame,
                tick: frame,
            },
            &status,
        ),
    }
}

fn candidate_fingerprint(candidates: &[TimeleapCandidate]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    let mut add = |value: i64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
    };
    for candidate in candidates {
        add(candidate.frame1);
        add(candidate.frame1_tick);
        add(candidate.post_tick);
        add(candidate.pre_frame);
        add(candidate.pre_sfmt_position);
        add(candidate.pre_status.model.remain.len() as i64);
        for remain in &candidate.pre_status.model.remain {
            add(i64::from(*remain));
        }
        add(i64::from(candidate.pre_status.fidget_cd));
        add(candidate.post_frame);
        add(candidate.post_sfmt_position);
        add(candidate.post_status.model.remain.len() as i64);
        for remain in &candidate.post_status.model.remain {
            add(i64::from(*remain));
        }
        add(i64::from(candidate.post_status.fidget_cd));
    }
    hash
}

mod candidate_search;
mod consider_delay;
mod merge;
