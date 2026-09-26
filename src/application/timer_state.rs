use crate::application::defaults;
use crate::domain::timing::game_frames_to_seconds;
use crate::errors::{AppError, AppResult, ArithmeticOperation, InputField};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalTimerSegment {
    Offset(usize),
    Wait,
}

/// 通常の乱数調整で使う、単純な待機タイマー。
///
/// 入力はFrameで統一し、複数のオフセットを上から順番に計時した後、
/// 実際の待機Frameを計時する。孵化時瞬きの状態とは独立している。
pub struct NormalTimerState {
    pub wait_frames: String,
    pub offsets: Vec<String>,
    /// 各オフセットを計時へ含めるか。古い保存状態やテストで不足している
    /// 要素は有効として扱い、入力欄追加時に必ず同期する。
    pub offset_enabled: Vec<bool>,
    pub timer_start: Option<Instant>,
    pub timer_seconds: f64,
    pub timer_segments: Vec<(NormalTimerSegment, i64)>,
    pub timer_segment_index: usize,
    pub finished: bool,
}

impl Default for NormalTimerState {
    fn default() -> Self {
        Self {
            wait_frames: defaults::DEFAULT_WAIT_FRAMES.into(),
            offsets: Vec::new(),
            offset_enabled: Vec::new(),
            timer_start: None,
            timer_seconds: 0.0,
            timer_segments: Vec::new(),
            timer_segment_index: 0,
            finished: false,
        }
    }
}

impl NormalTimerState {
    pub fn is_running(&self) -> bool {
        self.timer_start.is_some()
    }

    pub fn remaining(&self) -> f64 {
        self.remaining_at(Instant::now())
    }

    pub(crate) fn remaining_at(&self, now: Instant) -> f64 {
        self.timer_start
            .map(|start| {
                (self.timer_seconds - now.saturating_duration_since(start).as_secs_f64()).max(0.0)
            })
            .unwrap_or(0.0)
    }

    pub fn total_frames(&self) -> AppResult<i64> {
        let wait = self
            .wait_frames
            .trim()
            .parse::<i64>()
            .map_err(|_| AppError::InvalidInteger(InputField::WaitFrame))?;
        if wait < 0 {
            return Err(AppError::NegativeValue(InputField::WaitFrame));
        }

        let mut total = wait;
        for (index, value) in self.offsets.iter().enumerate() {
            if !self.offset_is_enabled(index) {
                continue;
            }
            let offset = value
                .trim()
                .parse::<i64>()
                .map_err(|_| AppError::InvalidOffset { index })?;
            if offset < 0 {
                return Err(AppError::NegativeOffset { index });
            }
            total = total
                .checked_add(offset)
                .ok_or(AppError::ArithmeticOverflow(ArithmeticOperation::FrameSum))?;
        }
        if total < 0 {
            return Err(AppError::NegativeValue(InputField::WaitFrame));
        }
        Ok(total)
    }

    pub fn start(&mut self, fps: f64) -> AppResult<i64> {
        let total_frames = self.total_frames()?;
        if !fps.is_finite() || fps <= 0.0 {
            return Err(AppError::InvalidFps);
        }
        self.timer_segments = self.segment_plan()?;
        self.timer_segment_index = 0;
        self.timer_seconds = game_frames_to_seconds(
            self.timer_segments
                .first()
                .map(|(_, frames)| *frames)
                .unwrap_or(0) as f64,
            fps,
        );
        self.timer_start = Some(Instant::now());
        self.finished = false;
        Ok(total_frames)
    }

    pub fn segment_plan(&self) -> AppResult<Vec<(NormalTimerSegment, i64)>> {
        self.total_frames()?;
        let wait = self
            .wait_frames
            .trim()
            .parse::<i64>()
            .map_err(|_| AppError::InvalidInteger(InputField::WaitFrame))?;
        let mut segments = Vec::new();
        for (index, value) in self.offsets.iter().enumerate() {
            if !self.offset_is_enabled(index) {
                continue;
            }
            let frames = value
                .trim()
                .parse::<i64>()
                .map_err(|_| AppError::InvalidOffset { index })?;
            if frames < 0 {
                return Err(AppError::NegativeOffset { index });
            }
            segments.push((NormalTimerSegment::Offset(index), frames));
        }
        segments.push((NormalTimerSegment::Wait, wait));
        Ok(segments)
    }

    fn offset_is_enabled(&self, index: usize) -> bool {
        self.offset_enabled.get(index).copied().unwrap_or(true)
    }

    #[allow(dead_code)]
    pub fn current_segment(&self) -> Option<NormalTimerSegment> {
        self.timer_segments
            .get(self.timer_segment_index)
            .map(|(segment, _)| *segment)
    }

    pub fn next_segment_seconds(&self, fps: f64) -> Option<f64> {
        self.timer_segments
            .get(self.timer_segment_index.saturating_add(1))
            .map(|(_, frames)| game_frames_to_seconds(*frames as f64, fps))
    }

    /// 現在区間の厳密な終了時刻を引き継いで、次区間へ切り替える。
    #[cfg(test)]
    pub fn advance_segment(&mut self, fps: f64) -> bool {
        self.advance_segment_at(fps, Instant::now())
    }

    pub(crate) fn advance_segment_at(&mut self, fps: f64, now: Instant) -> bool {
        let next_start = self
            .timer_start
            .map(|start| start + Duration::from_secs_f64(self.timer_seconds))
            .unwrap_or(now);
        let next_index = self.timer_segment_index.saturating_add(1);
        let Some((_, frames)) = self.timer_segments.get(next_index).copied() else {
            self.complete();
            return false;
        };
        self.timer_segment_index = next_index;
        self.timer_seconds = game_frames_to_seconds(frames as f64, fps);
        self.timer_start = Some(next_start);
        true
    }

    pub fn stop(&mut self) {
        self.timer_start = None;
        self.timer_segments.clear();
        self.timer_segment_index = 0;
        self.finished = false;
    }

    pub fn complete(&mut self) {
        self.timer_start = None;
        self.finished = true;
    }
}
