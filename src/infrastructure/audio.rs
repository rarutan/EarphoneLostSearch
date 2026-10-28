//! 通知音の再生をOSの音声デバイスへ橋渡しする。
//!
//! 音声ストリームの保持とフォールバックをこの層へ閉じ込め、アプリケーション層が
//! 特定のオーディオバックエンドへ依存しないようにする。

use rodio::{
    buffer::SamplesBuffer, source::SineWave, OutputStream, OutputStreamHandle, Sink, Source,
};
use std::time::Duration;

const AUDIO_SAMPLE_RATE: u32 = 48_000;
const TIMER_BEEP_FREQUENCY_HZ: f32 = 880.0;
const TIMER_BEEP_DURATION_MS: usize = 120;
const TIMER_BEEP_FADE_OUT_MS: usize = 8;

/// 音声デバイスを保持し、アプリ内の通知音を再生するアダプター。
pub(crate) struct AudioOutput {
    // ストリームを保持しておかないと、beep関数の終了時に音声出力も終了する。
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl AudioOutput {
    pub(crate) fn open() -> Option<Self> {
        OutputStream::try_default()
            .ok()
            .map(|(_stream, handle)| Self { _stream, handle })
    }

    pub(crate) fn beep(&self) -> bool {
        let Ok(sink) = Sink::try_new(&self.handle) else {
            return false;
        };
        sink.append(SamplesBuffer::new(
            1,
            AUDIO_SAMPLE_RATE,
            timer_beep_samples(),
        ));
        sink.detach();
        true
    }

    pub(crate) fn candidate_confirmed(&self) -> bool {
        let Ok(sink) = Sink::try_new(&self.handle) else {
            return false;
        };
        sink.append(
            SineWave::new(660.0)
                .take_duration(Duration::from_millis(70))
                .amplify(0.65),
        );
        sink.append(
            SineWave::new(880.0)
                .take_duration(Duration::from_millis(80))
                .amplify(0.65),
        );
        sink.append(
            SineWave::new(1100.0)
                .take_duration(Duration::from_millis(170))
                .amplify(0.65),
        );
        sink.detach();
        true
    }

    pub(crate) fn candidate_not_found(&self) -> bool {
        let Ok(sink) = Sink::try_new(&self.handle) else {
            return false;
        };
        sink.append(
            SineWave::new(330.0)
                .take_duration(Duration::from_millis(120))
                .amplify(0.7),
        );
        sink.append(
            SineWave::new(220.0)
                .take_duration(Duration::from_millis(160))
                .amplify(0.7),
        );
        sink.detach();
        true
    }
}

fn timer_beep_samples() -> Vec<f32> {
    let sample_count = AUDIO_SAMPLE_RATE as usize * TIMER_BEEP_DURATION_MS / 1_000;
    let fade_sample_count = AUDIO_SAMPLE_RATE as usize * TIMER_BEEP_FADE_OUT_MS / 1_000;

    (0..sample_count)
        .map(|sample_index| {
            let phase =
                2.0 * std::f32::consts::PI * TIMER_BEEP_FREQUENCY_HZ * (sample_index + 1) as f32
                    / AUDIO_SAMPLE_RATE as f32;
            let gain = half_cosine_fade_out_gain(sample_index, sample_count, fade_sample_count);
            phase.sin() * gain
        })
        .collect()
}

fn half_cosine_fade_out_gain(
    sample_index: usize,
    sample_count: usize,
    fade_sample_count: usize,
) -> f32 {
    let fade_start = sample_count.saturating_sub(fade_sample_count);
    if sample_index < fade_start {
        return 1.0;
    }

    let fade_length = sample_count - fade_start;
    if fade_length <= 1 {
        return 0.0;
    }

    let progress = (sample_index - fade_start) as f32 / (fade_length - 1) as f32;
    0.5 * (1.0 + (std::f32::consts::PI * progress).cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_beep_keeps_its_original_waveform_until_the_fade() {
        let samples = timer_beep_samples();
        let expected_sample_count = AUDIO_SAMPLE_RATE as usize * TIMER_BEEP_DURATION_MS / 1_000;
        let fade_sample_count = AUDIO_SAMPLE_RATE as usize * TIMER_BEEP_FADE_OUT_MS / 1_000;
        let fade_start = expected_sample_count - fade_sample_count;

        assert_eq!(samples.len(), expected_sample_count);
        for (sample_index, &sample) in samples.iter().take(fade_start).enumerate() {
            let original_phase =
                2.0 * std::f32::consts::PI * TIMER_BEEP_FREQUENCY_HZ * (sample_index + 1) as f32
                    / AUDIO_SAMPLE_RATE as f32;
            assert_eq!(sample, original_phase.sin());
        }
    }

    #[test]
    fn timer_beep_fades_smoothly_to_silence_at_its_last_sample() {
        let sample_count = AUDIO_SAMPLE_RATE as usize * TIMER_BEEP_DURATION_MS / 1_000;
        let fade_sample_count = AUDIO_SAMPLE_RATE as usize * TIMER_BEEP_FADE_OUT_MS / 1_000;
        let fade_start = sample_count - fade_sample_count;
        let midpoint = fade_start + fade_sample_count / 2;

        assert_eq!(
            half_cosine_fade_out_gain(fade_start, sample_count, fade_sample_count),
            1.0
        );
        assert!(
            (half_cosine_fade_out_gain(midpoint, sample_count, fade_sample_count) - 0.5).abs()
                < 0.01
        );
        assert_eq!(
            half_cosine_fade_out_gain(sample_count - 1, sample_count, fade_sample_count),
            0.0
        );
        assert_eq!(timer_beep_samples().last(), Some(&0.0));
    }
}

/// WASAPIが使えない場合のWindows標準音フォールバック。
#[cfg(windows)]
pub(crate) fn fallback_beep() {
    std::thread::spawn(|| unsafe {
        let _ = windows_sys::Win32::System::Diagnostics::Debug::Beep(880, 120);
    });
}

#[cfg(windows)]
pub(crate) fn fallback_candidate_not_found() {
    std::thread::spawn(|| unsafe {
        let _ = windows_sys::Win32::System::Diagnostics::Debug::Beep(330, 120);
        let _ = windows_sys::Win32::System::Diagnostics::Debug::Beep(220, 160);
    });
}

#[cfg(not(windows))]
pub(crate) fn fallback_beep() {}

#[cfg(not(windows))]
pub(crate) fn fallback_candidate_not_found() {}
