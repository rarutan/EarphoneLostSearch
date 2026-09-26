#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod application;
mod domain;
mod errors;
mod infrastructure;
mod ui;

// 下段パネルのボタン1つ分の余白を取り除いた固定ウィンドウ高。
const ACTION_BUTTON_HEIGHT: f32 = 32.0;
// 孵化条件枠のオフセット行右側に残っていた余白を取り除く。
const WINDOW_WIDTH: f32 = 1100.0 - ACTION_BUTTON_HEIGHT * 1.5;
// 孵化条件枠の開始位置行が下端へ食い込まないよう、半ボタン分だけ高さを確保する。
const WINDOW_HEIGHT: f32 = 660.0 - ACTION_BUTTON_HEIGHT * 1.2 + ACTION_BUTTON_HEIGHT * 0.5;

fn main() -> eframe::Result {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_min_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_max_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_resizable(false);

    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/earphone_lost_search_amber_cyan.png"
    ))) {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        "EarphoneLostSearch",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(ui::UiApp::new(cc)))),
    )
}

#[cfg(test)]
mod remain_simulation {
    use super::domain::rng::{ModelStatus, Sfmt};
    use rayon::prelude::*;
    use std::fmt::Write as _;
    use std::io::Write as _;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DrawTrace {
        stream_position: i64,
        model: usize,
        remain_before: i32,
        remain_after: i32,
        consumed: bool,
    }

    /// 1tickの途中にある「次に処理するモデル」から、1モデル分だけ進める。
    /// 連続Timeline上のSFMT位置480と481を比較するためのテスト専用処理であり、
    /// 通常の実装結果は同じtickをまとめて処理するModelStatus::next_stateで検証する。
    fn advance_one_model(
        status: &mut ModelStatus,
        model: usize,
        sfmt: &mut Sfmt,
        stream_position: &mut i64,
    ) -> DrawTrace {
        let remain_before = status.remain[model];
        let remain_after;
        let consumed;
        if status.remain[model] > 1 {
            status.remain[model] -= 1;
            remain_after = status.remain[model];
            consumed = false;
        } else if status.remain[model] < 0 {
            status.remain[model] += 1;
            if status.remain[model] == 0 {
                status.remain[model] = if sfmt.next_u64() % 3 == 0 { 36 } else { 30 };
                *stream_position += 1;
                consumed = true;
            } else {
                consumed = false;
            }
            remain_after = status.remain[model];
        } else {
            let value = sfmt.next_u64();
            *stream_position += 1;
            if value & 0x7f == 0 {
                status.remain[model] = -5;
            }
            remain_after = status.remain[model];
            consumed = true;
        }
        DrawTrace {
            stream_position: stream_position.saturating_sub(consumed as i64),
            model,
            remain_before,
            remain_after,
            consumed,
        }
    }

    /// 480番目のSFMT出力を起点に、SFMT位置が1000付近へ到達するまで
    /// 同じTimelineを連続して進め、各tickの消費範囲と全モデルのremainを記録する。
    ///
    /// 開始位置ごとにModelStatusを初期化する比較ではなく、1本のTimelineを
    /// 継続して観測するための回帰用シミュレーションである。
    #[test]
    fn dump_continuous_remain_from_480_to_1000() {
        const SEED: u32 = 0x0000_0001;
        const MODELS: usize = 10;
        const START_SFMT: i64 = 480;
        const END_SFMT: i64 = 1_000;

        let mut sfmt = Sfmt::new(SEED);
        for _ in 0..START_SFMT {
            sfmt.next_u64();
        }

        let mut status = ModelStatus::new(MODELS);
        let mut stream_position = START_SFMT;
        let mut tick = 0_i64;
        let mut output = String::new();
        writeln!(
            output,
            "seed={SEED:08X} models={MODELS} start_sfmt={START_SFMT} target_sfmt={END_SFMT}"
        )
        .unwrap();
        output.push_str(
            "tick,stream_before,stream_after,used,blink_models,remain_before,remain_after\n",
        );

        while stream_position < END_SFMT {
            let stream_before = stream_position;
            let remain_before = status.remain.clone();
            let (used, blink_models) = status.next_state(&mut sfmt);
            stream_position += used as i64;
            let remain_after = status.remain.clone();
            writeln!(
                output,
                "{tick},{stream_before},{stream_position},{used},{blink_models:?},{remain_before:?},{remain_after:?}"
            )
            .unwrap();
            tick += 1;
        }

        writeln!(
            output,
            "summary=ticks:{tick} final_sfmt:{stream_position} final_remain:{:?}",
            status.remain
        )
        .unwrap();

        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_480_to_1000.csv", output).unwrap();
    }

    /// 480開始の連続Timelineから、SFMT[481]を処理する直前の状態を切り出し、
    /// その状態から1000付近まで再生したログを別ファイルへ出力する。
    ///
    /// これにより、480開始側を独立初期化した比較ではなく、480の処理結果を
    /// 481開始側へ引き継いだ場合のremainと消費位置を確認できる。
    #[test]
    fn dump_continuous_cursor_views_from_480_to_1000() {
        const SEED: u32 = 0x0000_0001;
        const MODELS: usize = 10;
        const START_SFMT: i64 = 480;
        const END_SFMT: i64 = 1_000;

        let mut sfmt = Sfmt::new(SEED);
        for _ in 0..START_SFMT {
            sfmt.next_u64();
        }
        let mut status = ModelStatus::new(MODELS);
        let mut stream_position = START_SFMT;
        let mut tick = 0_i64;
        let mut model_cursor = 0_usize;
        let mut output = String::new();
        writeln!(
            output,
            "seed={SEED:08X} models={MODELS} continuous_start={START_SFMT} target_sfmt={END_SFMT}"
        )
        .unwrap();
        output.push_str(
            "tick,stream_position,model,remain_before,remain_after,consumed,remain_vector\n",
        );

        while stream_position < END_SFMT {
            let trace =
                advance_one_model(&mut status, model_cursor, &mut sfmt, &mut stream_position);
            writeln!(
                output,
                "{tick},{},{},{},{},{}, {:?}",
                trace.stream_position,
                trace.model,
                trace.remain_before,
                trace.remain_after,
                trace.consumed,
                status.remain
            )
            .unwrap();
            model_cursor += 1;
            if model_cursor == MODELS {
                model_cursor = 0;
                tick += 1;
            }
        }

        writeln!(
            output,
            "summary=ticks:{tick} final_sfmt:{stream_position} final_remain:{:?}",
            status.remain
        )
        .unwrap();
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_cursor_480_to_1000.csv", output).unwrap();
    }

    struct IndependentRun {
        log: String,
        blink_ticks: Vec<i64>,
        target_remains: Vec<i32>,
    }

    /// 指定したSFMT位置から、それぞれ別のTimelineとしてremain=0で開始する。
    /// 480/idx1と481/idx0の対象モデル列を、同じモデル数・同じ終了位置で比較する。
    fn simulate_independent(
        seed: u32,
        start_sfmt: i64,
        models: usize,
        target_model: usize,
        end_sfmt: i64,
    ) -> IndependentRun {
        let mut sfmt = Sfmt::new(seed);
        for _ in 0..start_sfmt {
            sfmt.next_u64();
        }
        let mut status = ModelStatus::new(models);
        let mut stream_position = start_sfmt;
        let mut tick = 0_i64;
        let mut blink_ticks = Vec::new();
        let mut target_remains = Vec::new();
        let mut log = String::new();
        writeln!(
            log,
            "seed={seed:08X} models={models} start_sfmt={start_sfmt} target_model={target_model} end_sfmt={end_sfmt}"
        )
        .unwrap();
        log.push_str(
            "tick,stream_before,stream_after,used,target_remain_before,target_remain_after,target_blinked,remain_vector\n",
        );

        while stream_position < end_sfmt {
            let stream_before = stream_position;
            let target_remain_before = status.remain[target_model];
            let (used, blink_models) = status.next_state(&mut sfmt);
            stream_position += used as i64;
            let target_blinked = blink_models.contains(&(target_model as u8));
            if target_blinked {
                blink_ticks.push(tick);
            }
            target_remains.push(status.remain[target_model]);
            writeln!(
                log,
                "{tick},{stream_before},{stream_position},{used},{target_remain_before},{},{target_blinked},{:?}",
                status.remain[target_model],
                status.remain
            )
            .unwrap();
            tick += 1;
        }
        writeln!(
            log,
            "summary=ticks:{tick} final_sfmt:{stream_position} blink_ticks:{blink_ticks:?}"
        )
        .unwrap();
        IndependentRun {
            log,
            blink_ticks,
            target_remains,
        }
    }

    /// 480開始・idx1と481開始・idx0を別Timelineとして実行し、
    /// 1000付近までの対象モデルの瞬きイベントを比較する。
    #[test]
    fn compare_independent_timelines_480_idx1_vs_481_idx0() {
        const SEED: u32 = 0x0000_024a;
        const MODELS: usize = 10;
        const END_SFMT: i64 = 1_000;
        let first = simulate_independent(SEED, 480, MODELS, 1, END_SFMT);
        let second = simulate_independent(SEED, 481, MODELS, 0, END_SFMT);

        let first_difference = first
            .blink_ticks
            .iter()
            .zip(second.blink_ticks.iter())
            .position(|(left, right)| left != right);
        let first_target_state_difference = first
            .target_remains
            .iter()
            .zip(second.target_remains.iter())
            .position(|(left, right)| left != right);
        let mut output = String::new();
        writeln!(
            output,
            "comparison=independent start480/idx1 vs start481/idx0 first_blink_tick_difference={first_difference:?} first_target_remain_difference={first_target_state_difference:?}"
        )
        .unwrap();
        output.push_str("\n--- start480 idx1 ---\n");
        output.push_str(&first.log);
        output.push_str("\n--- start481 idx0 ---\n");
        output.push_str(&second.log);
        std::fs::create_dir_all("target").unwrap();
        std::fs::write(
            "target/remain_simulation_independent_480_idx1_vs_481_idx0.txt",
            output,
        )
        .unwrap();
    }

    /// 比較対象のremainが実際に変化する固定Seedを探すための探索用テスト。
    #[test]
    fn find_seed_with_independent_idx_difference() {
        const MODELS: usize = 10;
        const END_SFMT: i64 = 1_000;
        for seed in 1..=10_000_u32 {
            let first = simulate_independent(seed, 480, MODELS, 1, END_SFMT);
            let second = simulate_independent(seed, 481, MODELS, 0, END_SFMT);
            let target_difference = first
                .target_remains
                .iter()
                .zip(second.target_remains.iter())
                .any(|(left, right)| left != right);
            if target_difference {
                let mut output = String::new();
                let target_difference_index = first
                    .target_remains
                    .iter()
                    .zip(second.target_remains.iter())
                    .position(|(left, right)| left != right);
                writeln!(
                    output,
                    "selected_seed={seed:08X} target_difference_index={target_difference_index:?}"
                )
                .unwrap();
                output.push_str("\n--- start480 idx1 ---\n");
                output.push_str(&first.log);
                output.push_str("\n--- start481 idx0 ---\n");
                output.push_str(&second.log);
                std::fs::create_dir_all("target").unwrap();
                std::fs::write("target/remain_simulation_selected_seed.txt", output).unwrap();
                return;
            }
        }
        panic!("no demonstrative seed found");
    }

    struct CompactRun {
        blink_ticks: Vec<i64>,
        target_remains: Vec<i32>,
    }

    fn simulate_independent_compact(
        seed: u32,
        start_sfmt: i64,
        models: usize,
        target_model: usize,
        end_sfmt: i64,
    ) -> CompactRun {
        let mut sfmt = Sfmt::new(seed);
        for _ in 0..start_sfmt {
            sfmt.next_u64();
        }
        let mut status = ModelStatus::new(models);
        let mut stream_position = start_sfmt;
        let mut tick = 0_i64;
        let mut blink_ticks = Vec::new();
        let mut target_remains = Vec::new();
        while stream_position < end_sfmt {
            let (used, blink_models) = status.next_state(&mut sfmt);
            stream_position += used as i64;
            if blink_models.contains(&(target_model as u8)) {
                blink_ticks.push(tick);
            }
            target_remains.push(status.remain[target_model]);
            tick += 1;
        }
        CompactRun {
            blink_ticks,
            target_remains,
        }
    }

    /// 複数のSeed・モデル数で、開始位置を1つずらした独立Timelineの
    /// 対象モデル列がどの程度一致し続けるかを集計する。
    #[test]
    fn summarize_independent_timeline_divergence() {
        const END_SFMT: i64 = 1_000;
        const SEED_COUNT: u32 = 1_000;
        let model_counts = [1_usize, 5, 10, 20];
        let mut output = String::from(
            "models,seeds,identical_blink_sequence,identical_remain_sequence,first_tick_divergence_at_or_before_10,first_tick_divergence_at_or_before_100\n",
        );
        for models in model_counts {
            let target_first = if models == 1 { 0 } else { 1 };
            let target_second = 0;
            let mut identical_blinks = 0_u32;
            let mut identical_remains = 0_u32;
            let mut diverged_by_10 = 0_u32;
            let mut diverged_by_100 = 0_u32;
            for seed in 1..=SEED_COUNT {
                let first = simulate_independent_compact(seed, 480, models, target_first, END_SFMT);
                let second =
                    simulate_independent_compact(seed, 481, models, target_second, END_SFMT);
                if first.blink_ticks == second.blink_ticks {
                    identical_blinks += 1;
                }
                if first.target_remains == second.target_remains {
                    identical_remains += 1;
                }
                let first_divergence = first
                    .target_remains
                    .iter()
                    .zip(second.target_remains.iter())
                    .position(|(left, right)| left != right);
                if first_divergence.is_some_and(|tick| tick <= 10) {
                    diverged_by_10 += 1;
                }
                if first_divergence.is_some_and(|tick| tick <= 100) {
                    diverged_by_100 += 1;
                }
            }
            writeln!(
                output,
                "{models},{SEED_COUNT},{identical_blinks},{identical_remains},{diverged_by_10},{diverged_by_100}"
            )
            .unwrap();
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_divergence_summary.csv", output).unwrap();
    }

    fn simulate_independent_for_ticks(
        seed: u32,
        start_sfmt: i64,
        models: usize,
        target_model: usize,
        ticks: usize,
    ) -> CompactRun {
        let mut sfmt = Sfmt::new(seed);
        for _ in 0..start_sfmt {
            sfmt.next_u64();
        }
        let mut status = ModelStatus::new(models);
        let mut blink_ticks = Vec::new();
        let mut target_remains = Vec::with_capacity(ticks);
        for tick in 0..ticks {
            let (_, blink_models) = status.next_state(&mut sfmt);
            if blink_models.contains(&(target_model as u8)) {
                blink_ticks.push(tick as i64);
            }
            target_remains.push(status.remain[target_model]);
        }
        CompactRun {
            blink_ticks,
            target_remains,
        }
    }

    /// SFMTの絶対終点による打ち切りの影響を避け、固定tick数で比較する。
    /// 480/idx1と481/idx0の対象モデルが、非対象モデルの抽選をきっかけに
    /// どの程度分岐するかをSeedごとに集計する。
    #[test]
    fn summarize_independent_timeline_divergence_fixed_ticks() {
        const TICKS: usize = 100;
        const SEED_COUNT: u32 = 10_000;
        let model_counts = [5_usize, 10, 20];
        let mut output = String::from(
            "models,ticks,seeds,identical_blink_sequence,identical_target_remain_sequence,first_target_difference_by_tick_10,first_target_difference_by_tick_100\n",
        );
        for models in model_counts {
            let mut identical_blinks = 0_u32;
            let mut identical_target_remains = 0_u32;
            let mut differ_by_10 = 0_u32;
            let mut differ_by_100 = 0_u32;
            for seed in 1..=SEED_COUNT {
                let first = simulate_independent_for_ticks(seed, 480, models, 1, TICKS);
                let second = simulate_independent_for_ticks(seed, 481, models, 0, TICKS);
                if first.blink_ticks == second.blink_ticks {
                    identical_blinks += 1;
                }
                if first.target_remains == second.target_remains {
                    identical_target_remains += 1;
                }
                let first_difference = first
                    .target_remains
                    .iter()
                    .zip(second.target_remains.iter())
                    .position(|(left, right)| left != right);
                if first_difference.is_some_and(|tick| tick < 10) {
                    differ_by_10 += 1;
                }
                if first_difference.is_some_and(|tick| tick < TICKS) {
                    differ_by_100 += 1;
                }
            }
            writeln!(
                output,
                "{models},{TICKS},{SEED_COUNT},{identical_blinks},{identical_target_remains},{differ_by_10},{differ_by_100}"
            )
            .unwrap();
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_fixed_tick_summary.csv", output).unwrap();
    }

    /// 開始位置と対象モデル番号の差を1〜4へ変えた場合も確認する。
    #[test]
    fn summarize_index_shift_pairs() {
        const MODELS: usize = 20;
        const TICKS: usize = 100;
        const SEED_COUNT: u32 = 10_000;
        let mut output = String::from("delta,seeds,identical_target_remain_sequence\n");
        for delta in 1..=4_usize {
            let mut identical = 0_u32;
            for seed in 1..=SEED_COUNT {
                let first = simulate_independent_for_ticks(seed, 480, MODELS, delta, TICKS);
                let second =
                    simulate_independent_for_ticks(seed, 480 + delta as i64, MODELS, 0, TICKS);
                if first.target_remains == second.target_remains {
                    identical += 1;
                }
            }
            writeln!(output, "{delta},{SEED_COUNT},{identical}").unwrap();
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_index_shift_summary.csv", output).unwrap();
    }

    #[derive(Clone, Copy, Debug)]
    struct LongPairSummary {
        first_extra_trigger_tick: Option<i64>,
        first_extra_trigger_sfmt: Option<i64>,
        first_trailing_trigger_tick: Option<i64>,
        first_trailing_trigger_sfmt: Option<i64>,
        first_target_state_difference_tick: Option<i64>,
        first_target_blink_difference_tick: Option<i64>,
        final_sfmt_a: i64,
        final_sfmt_b: i64,
        ticks: i64,
    }

    /// 480/idx1と481/idx0を、それぞれ独立Timeline（全remain=0）として
    /// 10,000,000F付近まで走査する。A側のmodel0はB側に対応するモデルが
    /// ないため、その抽選が後から`0 -> -5`へ入る時点を別記録する。
    ///
    /// SFMTの絶対位置ではなく同じtick番号で対象モデルを比較することで、
    /// 「余分なモデルの分岐がいつ対象モデルへ波及したか」を確認する。
    fn compare_shifted_pair_to_sfmt_limit(
        seed: u32,
        start_a: i64,
        start_b: i64,
        models: usize,
        end_sfmt_a: i64,
        end_sfmt_b: i64,
    ) -> LongPairSummary {
        let mut sfmt_a = Sfmt::new(seed);
        for _ in 0..start_a {
            sfmt_a.next_u64();
        }
        let mut sfmt_b = Sfmt::new(seed);
        for _ in 0..start_b {
            sfmt_b.next_u64();
        }
        let mut status_a = ModelStatus::new(models);
        let mut status_b = ModelStatus::new(models);
        let mut position_a = start_a;
        let mut position_b = start_b;
        let mut tick = 0_i64;
        let mut first_extra_trigger_tick = None;
        let mut first_extra_trigger_sfmt = None;
        let mut first_trailing_trigger_tick = None;
        let mut first_trailing_trigger_sfmt = None;
        let mut first_target_state_difference_tick = None;
        let mut first_target_blink_difference_tick = None;

        while position_a < end_sfmt_a && position_b < end_sfmt_b {
            let extra_before = status_a.remain[0];
            let trailing_before = status_b.remain[models - 1];
            let (used_a, blink_a) = status_a.next_state(&mut sfmt_a);
            let (used_b, blink_b) = status_b.next_state(&mut sfmt_b);
            if first_extra_trigger_tick.is_none() && extra_before == 0 && status_a.remain[0] == -5 {
                first_extra_trigger_tick = Some(tick);
                first_extra_trigger_sfmt = Some(position_a);
            }
            if first_trailing_trigger_tick.is_none()
                && trailing_before == 0
                && status_b.remain[models - 1] == -5
            {
                first_trailing_trigger_tick = Some(tick);
                first_trailing_trigger_sfmt = Some(position_b);
            }
            if first_target_state_difference_tick.is_none()
                && status_a.remain[1] != status_b.remain[0]
            {
                first_target_state_difference_tick = Some(tick);
            }
            let target_blinked_a = blink_a.contains(&1);
            let target_blinked_b = blink_b.contains(&0);
            if first_target_blink_difference_tick.is_none() && target_blinked_a != target_blinked_b
            {
                first_target_blink_difference_tick = Some(tick);
            }
            position_a += used_a as i64;
            position_b += used_b as i64;
            tick += 1;
        }

        LongPairSummary {
            first_extra_trigger_tick,
            first_extra_trigger_sfmt,
            first_trailing_trigger_tick,
            first_trailing_trigger_sfmt,
            first_target_state_difference_tick,
            first_target_blink_difference_tick,
            final_sfmt_a: position_a,
            final_sfmt_b: position_b,
            ticks: tick,
        }
    }

    /// 長距離での分岐位置を複数Seedについて記録する回帰テスト。
    /// 通常のテスト実行を遅くしないよう、必要時に明示実行する。
    #[test]
    #[ignore = "10,000,000Fまで走査する長時間シミュレーション"]
    fn scan_shifted_pair_to_ten_million_sfmt() {
        const MODELS: usize = 10;
        const START_A: i64 = 480;
        const START_B: i64 = 481;
        const END_A: i64 = 10_000_000;
        const END_B: i64 = 10_000_001;
        const SEED_COUNT: u32 = 256;
        let mut output = String::from(
            "seed,first_extra_trigger_tick,first_extra_trigger_sfmt,first_trailing_trigger_tick,first_trailing_trigger_sfmt,first_target_state_difference_tick,first_target_blink_difference_tick,final_sfmt_a,final_sfmt_b,ticks\n",
        );
        for seed in 1..=SEED_COUNT {
            let result =
                compare_shifted_pair_to_sfmt_limit(seed, START_A, START_B, MODELS, END_A, END_B);
            writeln!(
                output,
                "{seed:08X},{:?},{:?},{:?},{:?},{:?},{:?},{},{},{}",
                result.first_extra_trigger_tick,
                result.first_extra_trigger_sfmt,
                result.first_trailing_trigger_tick,
                result.first_trailing_trigger_sfmt,
                result.first_target_state_difference_tick,
                result.first_target_blink_difference_tick,
                result.final_sfmt_a,
                result.final_sfmt_b,
                result.ticks,
            )
            .unwrap();
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_shift_to_10m.csv", output).unwrap();
    }

    #[test]
    #[ignore = "10,000,000Fまで特定Seedを走査する長時間シミュレーション"]
    fn scan_known_divergent_seed_to_ten_million_sfmt() {
        const MODELS: usize = 10;
        let result = compare_shifted_pair_to_sfmt_limit(
            0x0000_024a,
            480,
            481,
            MODELS,
            10_000_000,
            10_000_001,
        );
        let output = format!(
            "seed=0000024A models={MODELS} first_extra_trigger_tick={:?} first_extra_trigger_sfmt={:?} first_trailing_trigger_tick={:?} first_trailing_trigger_sfmt={:?} first_target_state_difference_tick={:?} first_target_blink_difference_tick={:?} final_sfmt_a={} final_sfmt_b={} ticks={}\n",
            result.first_extra_trigger_tick,
            result.first_extra_trigger_sfmt,
            result.first_trailing_trigger_tick,
            result.first_trailing_trigger_sfmt,
            result.first_target_state_difference_tick,
            result.first_target_blink_difference_tick,
            result.final_sfmt_a,
            result.final_sfmt_b,
            result.ticks,
        );
        std::fs::create_dir_all("target").unwrap();
        std::fs::write(
            "target/remain_simulation_known_divergent_seed_to_10m.txt",
            output,
        )
        .unwrap();
    }

    #[test]
    #[ignore = "複数NPC数を10,000,000Fまで走査する長時間シミュレーション"]
    fn scan_shifted_pair_model_counts_to_ten_million_sfmt() {
        const START_A: i64 = 480;
        const START_B: i64 = 481;
        const END_A: i64 = 10_000_000;
        const END_B: i64 = 10_000_001;
        const SEEDS: [u32; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let mut output = String::from(
            "models,seed,first_extra_trigger_sfmt,first_trailing_trigger_sfmt,first_target_state_difference_tick,first_target_blink_difference_tick\n",
        );
        for models in [2_usize, 5, 10, 20] {
            for seed in SEEDS {
                let result = compare_shifted_pair_to_sfmt_limit(
                    seed, START_A, START_B, models, END_A, END_B,
                );
                writeln!(
                    output,
                    "{models},{seed:08X},{:?},{:?},{:?},{:?}",
                    result.first_extra_trigger_sfmt,
                    result.first_trailing_trigger_sfmt,
                    result.first_target_state_difference_tick,
                    result.first_target_blink_difference_tick,
                )
                .unwrap();
            }
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write(
            "target/remain_simulation_shift_model_counts_to_10m.csv",
            output,
        )
        .unwrap();
    }

    #[derive(Clone, Copy, Debug)]
    struct ArbitraryPairSummary {
        first_prefix_trigger_sfmt: Option<i64>,
        first_suffix_trigger_sfmt: Option<i64>,
        first_target_state_difference_tick: Option<i64>,
        first_target_blink_difference_tick: Option<i64>,
    }

    /// 開始位置の差と観測モデル番号を任意に指定して、対応する2本の
    /// 独立Timelineを比較する。開始位置差は対象idx差と一致させる。
    fn compare_arbitrary_pair_to_sfmt_limit(
        seed: u32,
        start_a: i64,
        start_b: i64,
        models: usize,
        target_a: usize,
        target_b: usize,
        end_sfmt_a: i64,
        end_sfmt_b: i64,
    ) -> ArbitraryPairSummary {
        let mut sfmt_a = Sfmt::new(seed);
        for _ in 0..start_a {
            sfmt_a.next_u64();
        }
        let mut sfmt_b = Sfmt::new(seed);
        for _ in 0..start_b {
            sfmt_b.next_u64();
        }
        let mut status_a = ModelStatus::new(models);
        let mut status_b = ModelStatus::new(models);
        let mut position_a = start_a;
        let mut position_b = start_b;
        let mut tick = 0_i64;
        let shift = start_b.saturating_sub(start_a).unsigned_abs() as usize;
        let mut first_prefix_trigger_sfmt = None;
        let mut first_suffix_trigger_sfmt = None;
        let mut first_target_state_difference_tick = None;
        let mut first_target_blink_difference_tick = None;

        while position_a < end_sfmt_a && position_b < end_sfmt_b {
            let mut prefix_zero_mask = 0_u32;
            for model in 0..shift {
                if status_a.remain[model] == 0 {
                    prefix_zero_mask |= 1 << model;
                }
            }
            let suffix_start = models.saturating_sub(shift);
            let mut suffix_zero_mask = 0_u32;
            for model in suffix_start..models {
                if status_b.remain[model] == 0 {
                    suffix_zero_mask |= 1 << (model - suffix_start);
                }
            }
            let (used_a, blink_a) = status_a.next_state(&mut sfmt_a);
            let (used_b, blink_b) = status_b.next_state(&mut sfmt_b);
            if first_prefix_trigger_sfmt.is_none()
                && (0..shift).any(|model| {
                    (prefix_zero_mask & (1 << model)) != 0 && status_a.remain[model] == -5
                })
            {
                first_prefix_trigger_sfmt = Some(position_a);
            }
            if first_suffix_trigger_sfmt.is_none()
                && (suffix_start..models).any(|model| {
                    (suffix_zero_mask & (1 << (model - suffix_start))) != 0
                        && status_b.remain[model] == -5
                })
            {
                first_suffix_trigger_sfmt = Some(position_b);
            }
            if first_target_state_difference_tick.is_none()
                && status_a.remain[target_a] != status_b.remain[target_b]
            {
                first_target_state_difference_tick = Some(tick);
            }
            let target_blinked_a = blink_a.contains(&(target_a as u8));
            let target_blinked_b = blink_b.contains(&(target_b as u8));
            if first_target_blink_difference_tick.is_none() && target_blinked_a != target_blinked_b
            {
                first_target_blink_difference_tick = Some(tick);
            }
            position_a += used_a as i64;
            position_b += used_b as i64;
            tick += 1;
        }

        ArbitraryPairSummary {
            first_prefix_trigger_sfmt,
            first_suffix_trigger_sfmt,
            first_target_state_difference_tick,
            first_target_blink_difference_tick,
        }
    }

    /// 1 Seed分のSFMT列を共有して、複数の開始位置・idx組を比較する。
    /// ケースごとにSFMTを再生成しないため、長距離検証の計算量を抑えられる。
    #[allow(dead_code)]
    fn compare_arbitrary_pair_from_values(
        values: &[u64],
        start_a: i64,
        start_b: i64,
        models: usize,
        target_a: usize,
        target_b: usize,
        end_sfmt_a: i64,
        end_sfmt_b: i64,
    ) -> ArbitraryPairSummary {
        let mut status_a = ModelStatus::new(models);
        let mut status_b = ModelStatus::new(models);
        let mut cursor_a = start_a as usize;
        let mut cursor_b = start_b as usize;
        let mut position_a = start_a;
        let mut position_b = start_b;
        let mut tick = 0_i64;
        let shift = start_b.saturating_sub(start_a).unsigned_abs() as usize;
        let mut first_prefix_trigger_sfmt = None;
        let mut first_suffix_trigger_sfmt = None;
        let mut first_target_state_difference_tick = None;
        let mut first_target_blink_difference_tick = None;

        while position_a < end_sfmt_a && position_b < end_sfmt_b {
            let mut prefix_zero_mask = 0_u32;
            for model in 0..shift {
                if status_a.remain[model] == 0 {
                    prefix_zero_mask |= 1 << model;
                }
            }
            let suffix_start = models.saturating_sub(shift);
            let mut suffix_zero_mask = 0_u32;
            for model in suffix_start..models {
                if status_b.remain[model] == 0 {
                    suffix_zero_mask |= 1 << (model - suffix_start);
                }
            }
            let (used_a, blink_a) = status_a.next_state_with(|| {
                let value = values[cursor_a];
                cursor_a += 1;
                value
            });
            let (used_b, blink_b) = status_b.next_state_with(|| {
                let value = values[cursor_b];
                cursor_b += 1;
                value
            });
            if first_prefix_trigger_sfmt.is_none()
                && (0..shift).any(|model| {
                    (prefix_zero_mask & (1 << model)) != 0 && status_a.remain[model] == -5
                })
            {
                first_prefix_trigger_sfmt = Some(position_a);
            }
            if first_suffix_trigger_sfmt.is_none()
                && (suffix_start..models).any(|model| {
                    (suffix_zero_mask & (1 << (model - suffix_start))) != 0
                        && status_b.remain[model] == -5
                })
            {
                first_suffix_trigger_sfmt = Some(position_b);
            }
            if first_target_state_difference_tick.is_none()
                && status_a.remain[target_a] != status_b.remain[target_b]
            {
                first_target_state_difference_tick = Some(tick);
            }
            let target_blinked_a = blink_a.contains(&(target_a as u8));
            let target_blinked_b = blink_b.contains(&(target_b as u8));
            if first_target_blink_difference_tick.is_none() && target_blinked_a != target_blinked_b
            {
                first_target_blink_difference_tick = Some(tick);
            }
            position_a += used_a as i64;
            position_b += used_b as i64;
            tick += 1;
        }

        ArbitraryPairSummary {
            first_prefix_trigger_sfmt,
            first_suffix_trigger_sfmt,
            first_target_state_difference_tick,
            first_target_blink_difference_tick,
        }
    }

    /// Seedをチャンク単位で並列走査し、各チャンクの結果を直ちに保存する。
    /// 途中終了しても完了済みチャンクの結果を確認できるようにする。
    #[test]
    #[ignore = "100,000 Seedを10,000,000Fまで走査する長時間シミュレーション"]
    fn scan_100k_seeds_shifted_pairs_to_ten_million_sfmt() {
        const SEED_CHUNK_SIZE: u32 = 1_000;
        const START_A: i64 = 480;
        const END_A: i64 = 10_000_000;
        const CASES: [(usize, usize, usize, u32); 8] = [
            (6, 5, 0, 10_000),
            (6, 3, 0, 5_000),
            (10, 5, 0, 100_000),
            (10, 9, 6, 10_000),
            (14, 5, 0, 10_000),
            (14, 9, 6, 10_000),
            (20, 5, 0, 10_000),
            (20, 9, 6, 10_000),
        ];
        let output_dir = "target/remain_simulation_100k_chunks";
        std::fs::create_dir_all(output_dir).unwrap();
        let progress_path = format!("{output_dir}/progress.csv");
        std::fs::write(
            &progress_path,
            "models,target_a,target_b,seed_start,seed_end,elapsed_seconds,no_state_difference,no_blink_difference,delayed_state_difference,delayed_blink_difference,max_delayed_blink_tick\n",
        )
        .unwrap();
        let started = std::time::Instant::now();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(16)
            .build()
            .unwrap();
        let case_paths: Vec<_> = CASES
            .iter()
            .map(|(models, target_a, target_b, _seed_count)| {
                let case_name = format!("models{models}_a{target_a}_b{target_b}");
                let case_path = format!("{output_dir}/{case_name}.csv");
                let delayed_path = format!("{output_dir}/{case_name}_delayed.csv");
                std::fs::write(
                    &case_path,
                    "seed_start,seed_end,elapsed_seconds,no_state_difference,no_blink_difference,delayed_state_difference,delayed_blink_difference,max_delayed_blink_tick\n",
                )
                .unwrap();
                std::fs::write(
                    &delayed_path,
                    "seed,first_prefix_trigger_sfmt,first_suffix_trigger_sfmt,first_target_state_difference_tick,first_target_blink_difference_tick\n",
                )
                .unwrap();
                (case_path, delayed_path)
            })
            .collect();

        for (case_index, &(models, target_a, target_b, seed_count)) in CASES.iter().enumerate() {
            let start_b = START_A + (target_a as i64 - target_b as i64);
            let end_b = END_A + (target_a as i64 - target_b as i64);
            for seed_start in (1..=seed_count).step_by(SEED_CHUNK_SIZE as usize) {
                let seed_end = (seed_start + SEED_CHUNK_SIZE - 1).min(seed_count);
                let summaries: Vec<_> = pool.install(|| {
                    (seed_start..=seed_end)
                        .into_par_iter()
                        .map(|seed| {
                            let result = compare_arbitrary_pair_to_sfmt_limit(
                                seed, START_A, start_b, models, target_a, target_b, END_A, end_b,
                            );
                            (seed, result)
                        })
                        .collect()
                });
                let no_state_difference = summaries
                    .iter()
                    .filter(|(_, result)| result.first_target_state_difference_tick.is_none())
                    .count();
                let no_blink_difference = summaries
                    .iter()
                    .filter(|(_, result)| result.first_target_blink_difference_tick.is_none())
                    .count();
                let delayed_state = summaries
                    .iter()
                    .filter(|(_, result)| {
                        result
                            .first_target_state_difference_tick
                            .is_some_and(|tick| tick > 1)
                    })
                    .count();
                let delayed_blink: Vec<_> = summaries
                    .iter()
                    .filter(|(_, result)| {
                        result
                            .first_target_blink_difference_tick
                            .is_some_and(|tick| tick > 1)
                    })
                    .collect();
                let max_delayed_blink_tick = delayed_blink
                    .iter()
                    .filter_map(|(_, result)| result.first_target_blink_difference_tick)
                    .max();
                let elapsed = started.elapsed().as_secs_f64();
                let line = format!(
                    "{seed_start},{seed_end},{elapsed:.3},{no_state_difference},{no_blink_difference},{delayed_state},{},{:?}\n",
                    delayed_blink.len(),
                    max_delayed_blink_tick,
                );
                let (case_path, delayed_path) = &case_paths[case_index];
                let mut case_file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(case_path)
                    .unwrap();
                case_file.write_all(line.as_bytes()).unwrap();
                if !delayed_blink.is_empty() {
                    let mut delayed_file = std::fs::OpenOptions::new()
                        .append(true)
                        .open(delayed_path)
                        .unwrap();
                    for (seed, result) in delayed_blink.iter().take(20) {
                        writeln!(
                            delayed_file,
                            "{seed:08X},{:?},{:?},{:?},{:?}",
                            result.first_prefix_trigger_sfmt,
                            result.first_suffix_trigger_sfmt,
                            result.first_target_state_difference_tick,
                            result.first_target_blink_difference_tick,
                        )
                        .unwrap();
                    }
                }
                let mut progress_file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&progress_path)
                    .unwrap();
                writeln!(
                    progress_file,
                    "{models},{target_a},{target_b},{seed_start},{seed_end},{elapsed:.3},{no_state_difference},{no_blink_difference},{delayed_state},{}, {:?}",
                    delayed_blink.len(),
                    max_delayed_blink_tick,
                )
                .unwrap();
            }
        }
    }

    /// idxずれを開始位置のずれとして表現した場合、瞬き位置の差が一時的に発生して
    /// 後で元へ戻るケースがあるかを集計する検証用テスト。
    #[test]
    #[ignore = "10,000 Seedの探索レポートを生成する長時間シミュレーション"]
    fn summarize_blink_position_drift_and_recovery() {
        const SEED_COUNT: u32 = 10_000;
        const TICKS: usize = 1_000;
        let mut output = String::from(
            "models,delta,seeds,comparable,temporary_drift_and_recovery,max_abs_difference,example_seed,example_differences\n",
        );
        for models in [5_usize, 10, 20] {
            for delta in 1..=4_usize {
                let mut comparable = 0_u32;
                let mut recovered = 0_u32;
                let mut max_abs_difference = 0_i64;
                let mut example_seed = None;
                let mut example_differences = Vec::new();
                for seed in 1..=SEED_COUNT {
                    let first = simulate_independent_for_ticks(seed, 480, models, delta, TICKS);
                    let second =
                        simulate_independent_for_ticks(seed, 480 + delta as i64, models, 0, TICKS);
                    let length = first.blink_ticks.len().min(second.blink_ticks.len());
                    if length == 0 {
                        continue;
                    }
                    comparable += 1;
                    let differences: Vec<i64> = first.blink_ticks[..length]
                        .iter()
                        .zip(&second.blink_ticks[..length])
                        .map(|(left, right)| left - right)
                        .collect();
                    let local_max = differences
                        .iter()
                        .map(|value| value.abs())
                        .max()
                        .unwrap_or(0);
                    max_abs_difference = max_abs_difference.max(local_max);
                    let has_nonzero = differences.iter().any(|value| *value != 0);
                    let has_recovery = differences
                        .iter()
                        .enumerate()
                        .any(|(index, value)| *value != 0 && differences[index + 1..].contains(&0));
                    if has_nonzero && has_recovery {
                        recovered += 1;
                        if example_seed.is_none() {
                            example_seed = Some(seed);
                            example_differences = differences;
                        }
                    }
                }
                writeln!(
                    output,
                    "{models},{delta},{SEED_COUNT},{comparable},{recovered},{max_abs_difference},{:?},{:?}",
                    example_seed.map(|seed| format!("{seed:08X}")),
                    example_differences,
                )
                .unwrap();
            }
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_blink_drift_recovery.csv", output).unwrap();
    }

    /// 瞬きイベントの発生tickを真偽列として比較し、短い不一致が後で
    /// 完全一致へ戻るケースを調べる検証用テスト。
    #[test]
    #[ignore = "10,000 Seedの探索レポートを生成する長時間シミュレーション"]
    fn summarize_blink_mask_temporary_drift() {
        const SEED_COUNT: u32 = 10_000;
        const TICKS: usize = 1_000;
        let mut output = String::from(
            "models,delta,seeds,identical,temporary_mismatch,small_temporary_mismatch,example_seed,example_indices\n",
        );
        for models in [5_usize, 10, 20] {
            for delta in 1..=4_usize {
                let mut identical = 0_u32;
                let mut temporary = 0_u32;
                let mut small_temporary = 0_u32;
                let mut example_seed = None;
                let mut example_indices = Vec::new();
                for seed in 1..=SEED_COUNT {
                    let first = simulate_independent_for_ticks(seed, 480, models, delta, TICKS);
                    let second =
                        simulate_independent_for_ticks(seed, 480 + delta as i64, models, 0, TICKS);
                    let mut first_mask = vec![false; TICKS];
                    let mut second_mask = vec![false; TICKS];
                    for tick in first.blink_ticks {
                        if (tick as usize) < TICKS {
                            first_mask[tick as usize] = true;
                        }
                    }
                    for tick in second.blink_ticks {
                        if (tick as usize) < TICKS {
                            second_mask[tick as usize] = true;
                        }
                    }
                    let mismatch_indices: Vec<usize> = first_mask
                        .iter()
                        .zip(&second_mask)
                        .enumerate()
                        .filter_map(|(tick, (left, right))| (left != right).then_some(tick))
                        .collect();
                    if mismatch_indices.is_empty() {
                        identical += 1;
                        continue;
                    }
                    let first_mismatch = mismatch_indices[0];
                    let last_mismatch = *mismatch_indices.last().unwrap();
                    if last_mismatch + 100 < TICKS {
                        temporary += 1;
                        let short_window = last_mismatch.saturating_sub(first_mismatch) <= 4;
                        if mismatch_indices.len() <= 4 && short_window {
                            small_temporary += 1;
                            if example_seed.is_none() {
                                example_seed = Some(seed);
                                example_indices = mismatch_indices;
                            }
                        }
                    }
                }
                writeln!(
                    output,
                    "{models},{delta},{SEED_COUNT},{identical},{temporary},{small_temporary},{:?},{:?}",
                    example_seed.map(|seed| format!("{seed:08X}")),
                    example_indices,
                )
                .unwrap();
            }
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_blink_mask_drift.csv", output).unwrap();
    }

    /// 一時的な真偽列差が出た代表ケースの周辺イベントを保存する検証用テスト。
    #[test]
    fn dump_blink_mask_temporary_drift_examples() {
        let cases = [
            (5_usize, 4_usize, 0x205d_u32),
            (10, 4, 0x799),
            (20, 1, 0x0b12),
        ];
        let mut output = String::new();
        for (models, delta, seed) in cases {
            let first = simulate_independent_for_ticks(seed, 480, models, delta, 500);
            let second = simulate_independent_for_ticks(seed, 480 + delta as i64, models, 0, 500);
            writeln!(output, "models={models} delta={delta} seed={seed:08X}").unwrap();
            writeln!(output, "idx_shifted={:?}", first.blink_ticks).unwrap();
            writeln!(output, "idx0={:?}", second.blink_ticks).unwrap();
            writeln!(output).unwrap();
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write(
            "target/remain_simulation_blink_mask_drift_examples.txt",
            output,
        )
        .unwrap();
    }

    /// 瞬きイベント数が同じ候補だけを対象に、位置差が1～2tick程度だけ発生して
    /// 後で0へ戻るケースを分離する検証用テスト。
    #[test]
    #[ignore = "10,000 Seedの探索レポートを生成する長時間シミュレーション"]
    fn summarize_equal_count_small_blink_position_drift() {
        const SEED_COUNT: u32 = 10_000;
        const TICKS: usize = 2_000;
        let mut output = String::from(
            "models,delta,seeds,equal_count,small_drift,small_drift_recovery,example_seed,example_differences\n",
        );
        for models in [5_usize, 10, 20] {
            for delta in 1..=4_usize {
                let mut equal_count = 0_u32;
                let mut small_drift = 0_u32;
                let mut recovery = 0_u32;
                let mut example_seed = None;
                let mut example_differences = Vec::new();
                for seed in 1..=SEED_COUNT {
                    let first = simulate_independent_for_ticks(seed, 480, models, delta, TICKS);
                    let second =
                        simulate_independent_for_ticks(seed, 480 + delta as i64, models, 0, TICKS);
                    if first.blink_ticks.len() != second.blink_ticks.len()
                        || first.blink_ticks.is_empty()
                    {
                        continue;
                    }
                    equal_count += 1;
                    let differences: Vec<i64> = first
                        .blink_ticks
                        .iter()
                        .zip(&second.blink_ticks)
                        .map(|(left, right)| left - right)
                        .collect();
                    let all_small = differences.iter().all(|difference| difference.abs() <= 2);
                    if !all_small || differences.iter().all(|difference| *difference == 0) {
                        continue;
                    }
                    small_drift += 1;
                    let has_recovery = differences.iter().enumerate().any(|(index, difference)| {
                        *difference != 0 && differences[index + 1..].contains(&0)
                    });
                    if has_recovery {
                        recovery += 1;
                        if example_seed.is_none() {
                            example_seed = Some(seed);
                            example_differences = differences;
                        }
                    }
                }
                writeln!(
                    output,
                    "{models},{delta},{SEED_COUNT},{equal_count},{small_drift},{recovery},{:?},{:?}",
                    example_seed.map(|seed| format!("{seed:08X}")),
                    example_differences,
                )
                .unwrap();
            }
        }
        std::fs::create_dir_all("target").unwrap();
        std::fs::write(
            "target/remain_simulation_equal_count_small_drift.csv",
            output,
        )
        .unwrap();
    }

    /// 開始位置の差分区間に `SFMT % 128 == 0` が存在しない候補だけを対象に、
    /// 同じNPC数で開始位置をずらした場合の瞬き列が一致し続けるかを確認する。
    ///
    /// これは「差分が総モデル数未満で、差分区間に特殊消費トリガーがないなら
    /// 以後の瞬き間隔は同一」という仮説の反例探索である。
    #[test]
    #[ignore = "長時間の理論検証スキャン。必要時は --ignored で実行"]
    fn verify_no_trigger_interval_preserves_blink_sequence() {
        const SEED_COUNT: u32 = 10_000;
        const BASE_START: i64 = 480;
        const TICKS: usize = 1_000;
        let mut output = String::from(
            "models,delta,seeds,eligible_no_mod128,blink_sequence_mismatch,example_seed\n",
        );

        for models in [5_usize, 10, 20] {
            for delta in 1..models {
                let mut eligible = 0_u32;
                let mut mismatch = 0_u32;
                let mut example_seed = None;

                for seed in 1..=SEED_COUNT {
                    let mut sfmt = Sfmt::new(seed);
                    for _ in 0..BASE_START {
                        sfmt.next_u64();
                    }
                    let no_trigger = (0..delta).all(|_| sfmt.next_u64() & 0x7f != 0);
                    if !no_trigger {
                        continue;
                    }
                    eligible += 1;

                    let first =
                        simulate_independent_for_ticks(seed, BASE_START, models, delta, TICKS);
                    let second = simulate_independent_for_ticks(
                        seed,
                        BASE_START + delta as i64,
                        models,
                        0,
                        TICKS,
                    );
                    if first.blink_ticks != second.blink_ticks {
                        mismatch += 1;
                        if example_seed.is_none() {
                            example_seed = Some(seed);
                        }
                    }
                }

                writeln!(
                    output,
                    "{models},{delta},{SEED_COUNT},{eligible},{mismatch},{:?}",
                    example_seed.map(|seed| format!("{seed:08X}")),
                )
                .unwrap();
            }
        }

        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/remain_simulation_no_trigger_interval.csv", output).unwrap();
    }

    /// 初期差分区間にトリガーがなくても、後続の対応レーンでトリガーが
    /// 発生した場合の状態差を確認する補助検証。
    #[test]
    fn find_no_gap_future_lane_trigger_difference() {
        const MODELS: usize = 5;
        const DELTA: usize = 1;
        const START: i64 = 480;
        const TICKS: usize = 500;
        for seed in 1..=10_000_u32 {
            let mut sfmt = Sfmt::new(seed);
            for _ in 0..START {
                sfmt.next_u64();
            }
            let gap = sfmt.next_u64();
            let mut future = Sfmt::new(seed);
            for _ in 0..(START + MODELS as i64) {
                future.next_u64();
            }
            let future_value = future.next_u64();
            if gap & 0x7f == 0 || future_value & 0x7f != 0 {
                continue;
            }
            let left = simulate_independent_for_ticks(seed, START, MODELS, DELTA, TICKS);
            let right =
                simulate_independent_for_ticks(seed, START + DELTA as i64, MODELS, 0, TICKS);
            if left.blink_ticks != right.blink_ticks || left.target_remains != right.target_remains
            {
                let mut output = String::new();
                writeln!(
                    output,
                    "seed={seed:08X} gap={gap:016X} future={future_value:016X}"
                )
                .unwrap();
                writeln!(output, "left_blinks={:?}", left.blink_ticks).unwrap();
                writeln!(output, "right_blinks={:?}", right.blink_ticks).unwrap();
                std::fs::write("target/remain_simulation_no_gap_future_trigger.txt", output)
                    .unwrap();
                return;
            }
        }
    }
}
