//! 実時間、実機Frame、瞬き判定tickの換算を一か所に集約する。
//!
//! Target Frame（SFMT消費位置）は時間の単位ではないため、このモジュールでは
//! 扱わない。実機FrameとSFMT位置を同じ整数として換算しないこと。

pub const DEFAULT_FPS: f64 = 59.8621;
pub const GAME_FRAMES_PER_BLINK_TICK: i64 = 2;
pub(crate) const MAX_SCHEDULED_BEEPS: u32 = 1024;

fn valid_fps(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        DEFAULT_FPS
    }
}

/// 実機Frameを現実の秒数へ換算する。
pub fn game_frames_to_seconds(frames: f64, fps: f64) -> f64 {
    frames / valid_fps(fps)
}

/// 現実の秒数を実機Frameへ換算する。観測値を保持するため丸めない。
pub fn seconds_to_game_frames(seconds: f64, fps: f64) -> f64 {
    seconds * valid_fps(fps)
}

/// 約1/30秒ごとの瞬き判定tickを現実の秒数へ換算する。
pub fn blink_ticks_to_seconds(ticks: i64, fps: f64) -> f64 {
    game_frames_to_seconds(ticks.max(0) as f64 * GAME_FRAMES_PER_BLINK_TICK as f64, fps)
}

/// 現実の秒数を、乱数式と同じ約1/30秒の瞬き判定tickへ丸める。
#[allow(dead_code)]
pub fn seconds_to_blink_ticks(seconds: f64, fps: f64) -> i64 {
    (seconds_to_game_frames(seconds.max(0.0), fps) / GAME_FRAMES_PER_BLINK_TICK as f64).round()
        as i64
}

/// 1区間の終了時刻を基準に、前段区間と重ならないbeepだけを逆算する。
///
/// 通知音の回数や間隔はUI設定ですが、区間内へ配置する計算自体は表示や
/// 音声デバイスに依存しないため、時間ユーティリティとしてここに置く。
pub(crate) fn beep_offsets_for_segment(
    duration_seconds: f64,
    count: u32,
    interval: f64,
) -> Vec<f64> {
    if count == 0 {
        return Vec::new();
    }
    let duration = if duration_seconds.is_finite() {
        duration_seconds.max(0.0)
    } else {
        0.0
    };
    let interval = if interval.is_finite() && interval > 0.0 {
        interval
    } else {
        1.0
    };
    let slots = (duration / interval).floor();
    let available = if slots.is_finite() && slots >= 0.0 {
        slots.min(f64::from(MAX_SCHEDULED_BEEPS.saturating_sub(1))) as u32 + 1
    } else {
        1
    };
    let actual_count = count.min(available).min(MAX_SCHEDULED_BEEPS);
    let first = duration - interval * f64::from(actual_count.saturating_sub(1));
    (0..actual_count)
        .map(|index| first + interval * f64::from(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_tick_round_trip_uses_two_game_frames() {
        let seconds = blink_ticks_to_seconds(180, DEFAULT_FPS);
        assert_eq!(seconds_to_blink_ticks(seconds, DEFAULT_FPS), 180);
        assert!((seconds - 360.0 / DEFAULT_FPS).abs() < f64::EPSILON);
    }

    #[test]
    fn target_position_is_not_part_of_time_conversion() {
        assert!((game_frames_to_seconds(300.0, 60.0) - 5.0).abs() < f64::EPSILON);
        assert!((seconds_to_game_frames(5.0, 60.0) - 300.0).abs() < f64::EPSILON);
    }

    #[test]
    fn beeps_are_counted_backwards_from_segment_end() {
        let offsets = beep_offsets_for_segment(5.0, 6, 0.5);
        assert_eq!(offsets, vec![2.5, 3.0, 3.5, 4.0, 4.5, 5.0]);
    }

    #[test]
    fn short_segment_drops_beeps_that_would_overlap_previous_segment() {
        let offsets = beep_offsets_for_segment(1.0, 6, 0.5);
        assert_eq!(offsets, vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn zero_length_segment_still_beeps_once_at_its_boundary() {
        let offsets = beep_offsets_for_segment(0.0, 6, 0.5);
        assert_eq!(offsets, vec![0.0]);
    }

    #[test]
    fn blink_notification_uses_one_beep_even_with_repeated_beep_settings() {
        let offsets = beep_offsets_for_segment(5.0, 1, 0.5);
        assert_eq!(offsets, vec![5.0]);
    }
}
