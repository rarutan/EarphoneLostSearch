//! 通知音の再生をOSの音声デバイスへ橋渡しする。
//!
//! 音声ストリームの保持とフォールバックをこの層へ閉じ込め、アプリケーション層が
//! 特定のオーディオバックエンドへ依存しないようにする。

use rodio::{source::SineWave, OutputStream, OutputStreamHandle, Sink, Source};
use std::time::Duration;

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
        sink.append(SineWave::new(880.0).take_duration(Duration::from_millis(120)));
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
