use crate::application::defaults;
use crate::domain::timing::game_frames_to_seconds;
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
    pub status: String,
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
            status: defaults::INITIAL_TIMER_STATUS.into(),
        }
    }
}

impl NormalTimerState {
    pub fn is_running(&self) -> bool {
        self.timer_start.is_some()
    }

    pub fn remaining(&self) -> f64 {
        self.timer_start
            .map(|start| (self.timer_seconds - start.elapsed().as_secs_f64()).max(0.0))
            .unwrap_or(0.0)
    }

    pub fn total_frames(&self) -> Result<i64, String> {
        let wait = self
            .wait_frames
            .trim()
            .parse::<i64>()
            .map_err(|_| "実際の待機Frameは整数で入力してください。".to_string())?;
        if wait < 0 {
            return Err("実際の待機Frameは0以上で入力してください。".into());
        }

        let mut total = wait;
        for (index, value) in self.offsets.iter().enumerate() {
            if !self.offset_is_enabled(index) {
                continue;
            }
            let offset = value
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("オフセット{}は整数で入力してください。", index + 1))?;
            if offset < 0 {
                return Err(format!(
                    "オフセット{}は0以上で入力してください。",
                    index + 1
                ));
            }
            total = total
                .checked_add(offset)
                .ok_or_else(|| "待機Frameの合計が大きすぎます。".to_string())?;
        }
        if total < 0 {
            return Err("オフセットを加えた合計Frameは0以上にしてください。".into());
        }
        Ok(total)
    }

    pub fn start(&mut self, fps: f64) -> Result<i64, String> {
        let total_frames = self.total_frames()?;
        if !fps.is_finite() || fps <= 0.0 {
            return Err("FPSは0より大きい数値で入力してください。".into());
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
        self.status = format!("通常タイマー開始：合計{}F", total_frames);
        Ok(total_frames)
    }

    pub fn segment_plan(&self) -> Result<Vec<(NormalTimerSegment, i64)>, String> {
        self.total_frames()?;
        let wait = self
            .wait_frames
            .trim()
            .parse::<i64>()
            .map_err(|_| "実際の待機Frameは整数で入力してください。".to_string())?;
        let mut segments = Vec::new();
        for (index, value) in self.offsets.iter().enumerate() {
            if !self.offset_is_enabled(index) {
                continue;
            }
            let frames = value
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("オフセット{}は整数で入力してください。", index + 1))?;
            if frames < 0 {
                return Err(format!(
                    "オフセット{}は0以上で入力してください。",
                    index + 1
                ));
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
    pub fn advance_segment(&mut self, fps: f64) -> bool {
        let next_start = self
            .timer_start
            .map(|start| start + Duration::from_secs_f64(self.timer_seconds))
            .unwrap_or_else(Instant::now);
        let next_index = self.timer_segment_index.saturating_add(1);
        let Some((segment, frames)) = self.timer_segments.get(next_index).copied() else {
            self.complete();
            return false;
        };
        self.timer_segment_index = next_index;
        self.timer_seconds = game_frames_to_seconds(frames as f64, fps);
        self.timer_start = Some(next_start);
        self.status = match segment {
            NormalTimerSegment::Offset(index) => {
                format!("オフセット{}を計時中：{}F", index + 1, frames)
            }
            NormalTimerSegment::Wait => format!("実際の待機を計時中：{}F", frames),
        };
        true
    }

    pub fn stop(&mut self) {
        self.timer_start = None;
        self.timer_segments.clear();
        self.timer_segment_index = 0;
        self.finished = false;
        self.status = "通常タイマーを停止しました。".into();
    }

    pub fn complete(&mut self) {
        self.timer_start = None;
        self.finished = true;
        self.status = "通常タイマー完了。".into();
    }
}
