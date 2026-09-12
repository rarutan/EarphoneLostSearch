//! フィールド観測のセッション状態と検索結果を管理する。
//!
//! UIから受け取った観測値を検索入力へ変換し、候補の確定、補正、
//! 本番タイマーへの引き渡しに必要な状態を一つのセッションとして保持する。

use super::*;
use crate::application::defaults;
use crate::application::state_types::SessionPhase;
use crate::application::status::StatusKind;

#[derive(Clone, Copy, PartialEq, Eq)]
// ---------------------------------------------------------------------------
// フィールドタブのセッション状態とUI向け遷移
// ---------------------------------------------------------------------------

pub enum FieldMode {
    Idle,
    Observing,
    Correction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldTimerKind {
    NextBlink,
    Target,
}

#[derive(Clone)]
pub struct FieldState {
    mode: FieldMode,
    /// 観測対象idxを指定するか、全idxを候補として扱うか。
    pub target_mode: FieldTargetMode,
    pub seed: String,
    pub range_start: String,
    pub range_end: String,
    /// 「start +」で検索終点を決めるための幅。
    pub range_after_start: String,
    /// タイムライン仕様のNPC数欄。主人公1体は`config`内で暗黙に加算する。
    pub npc_count: String,
    /// NPC欄を`始点~終点`として展開する検索モード。
    pub multiple_npc_search: bool,
    /// 観測対象idx。0は主人公、1以上はNPC。
    pub target_model: String,
    /// 実際に取得できたTargetFrame。idx自動判定に使用する。
    pub actual_frame: String,
    pub tolerance: String,
    pub target_consumption: String,
    /// Targetタイマーだけに適用する表示Frameオフセット。観測Timelineは変更しない。
    pub encounter_offset: String,
    pub use_offset: bool,
    /// TargetFrame欄を編集してから、Enterまたはフォーカス移動で再検索するための状態。
    pub target_dirty: bool,
    /// Targetだけを変更して、観測済みの候補を再評価している状態。
    /// この間も既存の瞬きタイマーは止めない。
    pub target_research_pending: bool,
    /// 明示的にEnterで補正へ進んだ後はTargetFrameを固定する。
    /// 観測結果確定直後からEnterまでは、瞬きタイマー実行中でも編集できる。
    target_input_locked: bool,
    pub observed: Vec<Instant>,
    pub intervals: Vec<i64>,
    pub results: Vec<FieldResult>,
    /// 指定範囲を照合して候補0件で終了した状態。追加のShiftでは再検索しない。
    pub search_exhausted: bool,
    /// 実測Frameから絞り込まれた観測対象idxの候補。
    pub inferred_indices: Vec<usize>,
    /// ずれ補正を最初に計算した結果を、同じ補正セッションで再利用する。
    pub correction_calculated: bool,
    pub correction_suggested_offset: Option<i64>,
    /// 指定idxで一致せず、別idxへフォールバックしたときの一致idx。
    pub idx_fallback_indices: Vec<usize>,
    /// idx指定時に、候補開始Frameのうち477以上で最も近いもの。
    pub nearest_timeline_frame: Option<i64>,
    pub selected_result: usize,
    timer_start: Option<Instant>,
    timer_seconds: f64,
    /// Targetタイマーが完了して表示を0へ固定している状態。
    timer_finished: bool,
    timer_kind: FieldTimerKind,
    /// ずれ検証へ入った瞬間のTimeline位置。補正中は観測アンカーの経過
    /// 時間ではなく、この位置を表のCurrentとして保持する。
    correction_timeline_frame: Option<i64>,
    /// 既に計算済みの次回以降の瞬き時刻。各瞬きのたびにTimelineを
    /// 再走査せず、観測タイマーを滑らかにつなぐために使う。
    blink_wait_queue: VecDeque<i64>,
    /// 右上表で前回スクロール追従した行。表示状態のリセットにも使う。
    pub table_last_followed_row: Option<usize>,
    /// 右上のTimeline表を現在位置へ自動追従させるか。初期値は有効。
    pub table_follow_current: bool,
    /// タイマーの音・表示位相を実機に合わせる表示Frame補正。
    pub adjust: i64,
    pub status: String,
    pub status_kind: StatusKind,
    timeline_sfmt: Option<Sfmt>,
    /// 現在位置表示専用の連続Timelineカーソル。表の先読み用カーソルとは
    /// 分離し、約1/30秒ごとのSFMT現在値をUIへ提供する。
    current_timeline_sfmt: Option<Sfmt>,
    current_timeline_cursor: Option<TimelineCursor>,
}

impl Default for FieldState {
    fn default() -> Self {
        Self {
            mode: FieldMode::Idle,
            target_mode: FieldTargetMode::Known,
            seed: defaults::SEED.into(),
            range_start: defaults::FIELD_RANGE_START.into(),
            range_end: defaults::FIELD_RANGE_END.into(),
            range_after_start: defaults::FIELD_RANGE_AFTER_START.into(),
            npc_count: defaults::FIELD_NPC_COUNT.into(),
            multiple_npc_search: false,
            target_model: defaults::FIELD_TARGET_MODEL.into(),
            actual_frame: String::new(),
            tolerance: defaults::FIELD_TOLERANCE.into(),
            target_consumption: String::new(),
            encounter_offset: defaults::FIELD_ENCOUNTER_OFFSET.into(),
            use_offset: defaults::DEFAULT_USE_OFFSET,
            target_dirty: false,
            target_research_pending: false,
            target_input_locked: false,
            observed: Vec::new(),
            intervals: Vec::new(),
            results: Vec::new(),
            search_exhausted: false,
            inferred_indices: Vec::new(),
            correction_calculated: false,
            correction_suggested_offset: None,
            idx_fallback_indices: Vec::new(),
            nearest_timeline_frame: None,
            selected_result: 0,
            timer_start: None,
            timer_seconds: 0.0,
            timer_finished: false,
            timer_kind: FieldTimerKind::NextBlink,
            correction_timeline_frame: None,
            blink_wait_queue: VecDeque::new(),
            table_last_followed_row: None,
            table_follow_current: defaults::DEFAULT_TABLE_FOLLOW_CURRENT,
            adjust: defaults::DEFAULT_ADJUST,
            status: defaults::INITIAL_FIELD_STATUS.into(),
            status_kind: StatusKind::Informational,
            timeline_sfmt: None,
            current_timeline_sfmt: None,
            current_timeline_cursor: None,
        }
    }
}

impl FieldState {
    pub(crate) fn set_status(&mut self, message: impl Into<String>, kind: StatusKind) {
        self.status = message.into();
        self.status_kind = kind;
    }

    pub(crate) fn set_info_status(&mut self, message: impl Into<String>) {
        self.set_status(message, StatusKind::Informational);
    }

    pub(crate) fn set_actionable_status(&mut self, message: impl Into<String>) {
        self.set_status(message, StatusKind::Error);
    }

    pub(crate) fn set_candidate_not_found_status(&mut self, message: impl Into<String>) {
        self.set_status(message, StatusKind::CandidateNotFound);
    }

    pub(crate) fn set_search_range_status(&mut self, message: impl Into<String>) {
        self.set_status(message, StatusKind::SearchRange);
    }

    pub(crate) fn clear_status(&mut self) {
        self.status.clear();
        self.status_kind = StatusKind::Informational;
    }

    pub(crate) fn status_is_field_notice(&self) -> bool {
        self.status_kind.is_field_notice()
    }

    fn enter_idle(&mut self) {
        self.mode = FieldMode::Idle;
    }

    fn enter_observing(&mut self) {
        self.mode = FieldMode::Observing;
    }

    /// フィールド側の内部状態を共通フェーズへ写像する。
    pub fn session_phase(&self) -> SessionPhase {
        if self.mode == FieldMode::Correction {
            return SessionPhase::Correction;
        }
        if self.timer_start.is_some() {
            return SessionPhase::Timer;
        }
        if self.timer_finished {
            return SessionPhase::Timer;
        }
        if self.mode == FieldMode::Observing {
            return SessionPhase::Observing;
        }
        if self.results.len() == 1 {
            return SessionPhase::TimelineReady;
        }
        SessionPhase::Input
    }

    pub fn can_edit_general_settings(&self) -> bool {
        matches!(
            self.session_phase(),
            SessionPhase::Input | SessionPhase::Observing | SessionPhase::TimelineReady
        )
    }

    /// 瞬き観測の範囲・許容差・観測対象は開始前に確定する。
    /// 観測後に変更する場合はリセットして新しいセッションを開始する。
    pub fn can_edit_observation_settings(&self) -> bool {
        self.session_phase() == SessionPhase::Input
    }

    pub fn can_edit_correction_inputs(&self) -> bool {
        self.session_phase() == SessionPhase::Correction
    }

    /// TargetFrameは観測結果確定後からEnterまでは編集でき、
    /// 明示的に補正モードへ進んだ後だけロックする。
    pub fn can_edit_target(&self) -> bool {
        !self.target_input_locked && self.mode != FieldMode::Correction
    }

    pub fn can_start_timer(&self) -> bool {
        self.session_phase() == SessionPhase::TimelineReady
    }

    pub fn is_observing(&self) -> bool {
        self.mode == FieldMode::Observing
    }

    pub fn is_correction_mode(&self) -> bool {
        self.mode == FieldMode::Correction
    }

    pub fn timer_is_running(&self) -> bool {
        self.timer_start.is_some()
    }

    pub fn timer_start_time(&self) -> Option<Instant> {
        self.timer_start
    }

    pub fn timer_kind(&self) -> FieldTimerKind {
        self.timer_kind
    }

    pub fn timer_is_finished(&self) -> bool {
        self.timer_finished
    }

    pub fn timer_seconds_value(&self) -> f64 {
        self.timer_seconds
    }

    /// 現在の瞬き区間を終了し、次の区間へ進めるための遷移。
    /// タイマーの開始時刻をUIから直接消去しないようにする。
    pub fn finish_blink_timer_segment(&mut self, fps: f64) -> Result<(), String> {
        self.timer_start = None;
        self.advance_blink_timer(fps)
    }

    /// Target区間を0秒で保持する終端遷移。以後の再開可否は、明示的な
    /// `toggle_timer_kind`または`enter_correction_mode`だけが決める。
    pub fn finish_target_timer(&mut self) {
        self.timer_finished = true;
        self.set_info_status("Target Frameのエンカウント時刻です。");
    }

    /// 検索結果が一意になったときの遷移を一箇所へ集約する。
    pub fn mark_timeline_ready(&mut self) {
        if self.results.len() == 1 && self.mode != FieldMode::Correction {
            self.enter_idle();
        }
    }

    /// 検索スレッドの結果を現在のセッションへ反映できる状態か返す。
    ///
    /// 一意候補を得た後にTargetだけを変更した再計算も受け付ける。
    /// 通常のキャンセル後に古いスレッド結果を受け取らない条件は維持する。
    pub fn accepts_search_result(&self) -> bool {
        if self.target_research_pending {
            return true;
        }
        self.mode == FieldMode::Observing
            || (self.mode == FieldMode::Idle
                && self.results.is_empty()
                && self.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS
                && self.timer_start.is_none())
    }

    pub fn is_observation_input_enabled(&self) -> bool {
        // 候補0件で検索が終わっても、孵化側と同じく追加の瞬き入力を
        // 受け付けて再検索できるようにする。一意候補だけは停止する。
        self.mode == FieldMode::Observing && self.results.len() != 1
    }

    /// 入力条件が変わったとき、古い検索結果を現在の条件へ流用しない。
    /// 観測した時刻列は残すので、Target変更などは同じ観測から再検索できる。
    pub fn invalidate_search_results(&mut self, status: impl Into<String>) {
        self.results.clear();
        self.target_research_pending = false;
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.timeline_sfmt = None;
        self.current_timeline_sfmt = None;
        self.current_timeline_cursor = None;
        self.set_info_status(status);
    }

    /// Targetだけを再計算する。観測候補と瞬きタイマーは保持し、Target
    /// の計算中も次の瞬き通知を止めない。
    pub fn prepare_target_research(&mut self, fps: f64, status: impl Into<String>) {
        if self.timer_kind == FieldTimerKind::Target {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            } else {
                self.timer_start = None;
            }
        }
        for result in &mut self.results {
            result.target_wait_frame = None;
            result.target_exact = None;
            result.target_frame_before = None;
            result.target_frame_after = None;
            result.target_blink_count = None;
        }
        self.target_research_pending = true;
        self.search_exhausted = false;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.set_info_status(status);
    }

    /// Target欄を空に戻した場合は、再検索せず既存Timelineを使い続ける。
    pub fn clear_target_prediction(&mut self, fps: f64) {
        self.target_research_pending = false;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        if self.timer_kind == FieldTimerKind::Target {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            }
        }
        for result in &mut self.results {
            result.target_wait_frame = None;
            result.target_exact = None;
            result.target_frame_before = None;
            result.target_frame_after = None;
            result.target_blink_count = None;
        }
    }

    pub fn refresh_nearest_timeline_frame(&mut self) {
        self.nearest_timeline_frame =
            if self.target_mode == FieldTargetMode::Known && self.results.len() == 1 {
                self.results
                    .iter()
                    .filter(|result| result.start_consumption >= MIN_TIMELINE_FRAME)
                    .map(|result| result.start_consumption)
                    .min()
            } else {
                None
            };
    }

    pub fn start_observation(&mut self) {
        self.enter_observing();
        self.observed.clear();
        self.intervals.clear();
        self.results.clear();
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.target_dirty = false;
        self.target_research_pending = false;
        self.target_input_locked = false;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.timeline_sfmt = None;
        self.current_timeline_sfmt = None;
        self.current_timeline_cursor = None;
        self.set_info_status("観測中：瞬きのたびにShiftを押してください。");
    }

    pub fn cancel_observation(&mut self) {
        self.enter_idle();
        self.observed.clear();
        self.intervals.clear();
        self.results.clear();
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.target_dirty = false;
        self.target_research_pending = false;
        self.target_input_locked = false;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.timeline_sfmt = None;
        self.current_timeline_sfmt = None;
        self.current_timeline_cursor = None;
        self.set_info_status("瞬き観測をキャンセルしました。");
    }

    /// 検索始点から指定幅を足して、検索終点を設定する。
    pub fn set_range_from_start(&mut self) {
        let Ok(start) = self.range_start.trim().parse::<i64>() else {
            return;
        };
        let Ok(span) = self.range_after_start.trim().parse::<i64>() else {
            return;
        };
        if !(0..=MAX_SEARCH_FRAME).contains(&start) || !(0..=MAX_SEARCH_SPAN).contains(&span) {
            return;
        }
        let Some(end) = start.checked_add(span) else {
            return;
        };
        if end > MAX_SEARCH_FRAME {
            return;
        }
        self.range_end = end.to_string();
    }

    /// 選択候補の現在のSFMT位置からカーソルを初期化する。
    /// 続くチャンクはこのカーソルを直接進め、元のファイルキャッシュを超えて
    /// Timelineを延長できるようにする。
    pub(crate) fn prepare_timeline_stream(&mut self) {
        if self.results.len() != 1
            || (self.timeline_sfmt.is_some()
                && self.current_timeline_sfmt.is_some()
                && self.current_timeline_cursor.is_some())
        {
            return;
        }
        let Some(result) = self.results.get(self.selected_result) else {
            return;
        };
        let Some(extension_cursor) = result.timeline_cursor.as_ref() else {
            return;
        };
        let current_cursor = result
            .target_cursor
            .as_ref()
            .unwrap_or(extension_cursor)
            .clone();
        let extension_cursor = extension_cursor.clone();
        let Ok(seed) = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16) else {
            return;
        };
        let mut extension_sfmt = Sfmt::new(seed);
        for _ in 0..extension_cursor.stream_cursor.max(0) {
            extension_sfmt.next_u64();
        }
        let mut current_sfmt = Sfmt::new(seed);
        for _ in 0..current_cursor.stream_cursor.max(0) {
            current_sfmt.next_u64();
        }
        self.current_timeline_sfmt = Some(current_sfmt);
        self.current_timeline_cursor = Some(current_cursor);
        self.timeline_sfmt = Some(extension_sfmt);
    }

    /// 観測アンカーから現在の1/30秒tickまでTimelineを進め、現在のSFMT
    /// 消費位置を返す。先読み用カーソルとは独立しているため、表の描画だけ
    /// では未来のTimeline状態を消費しない。
    pub fn current_sfmt_frame(&mut self, fps: f64) -> Option<i64> {
        let current_frame = self.current_timeline_frame(fps)?;
        let result = self.results.get(self.selected_result)?;
        let anchor_frame = result.last_wait_frame;
        let desired_tick = current_frame.max(anchor_frame).saturating_div(2);
        self.prepare_timeline_stream();
        let cursor = self.current_timeline_cursor.as_mut()?;
        let sfmt = self.current_timeline_sfmt.as_mut()?;
        if cursor.tick > desired_tick {
            return Some(cursor.consumption);
        }
        while cursor.tick < desired_tick {
            let (used, _) = cursor.status.next_state(sfmt);
            cursor.stream_cursor = cursor.stream_cursor.saturating_add(used as i64);
            cursor.consumption = cursor.consumption.saturating_add(used as i64);
            cursor.tick = cursor.tick.saturating_add(1);
            if cursor.consumption >= crate::domain::rng::MAX_SFMT_FRAME {
                break;
            }
        }
        Some(cursor.consumption)
    }

    /// 現在位置が先読み末尾へ近づいたとき、選択候補のTimelineを延長する。
    /// 1回の呼び出しでは有限チャンクだけを計算し、長時間待機でもUIスレッドに
    /// 大きな計算を集中させない。
    pub(crate) fn extend_timeline_if_needed(&mut self, fps: f64) -> bool {
        if self.results.len() != 1 {
            return false;
        }
        self.prepare_timeline_stream();
        if self.timeline_sfmt.is_none() {
            return false;
        }
        let Some(current) = self.current_timeline_frame(fps) else {
            return false;
        };
        let Some(result) = self.results.get(self.selected_result) else {
            return false;
        };
        let Some(last) = result.timeline.last() else {
            return false;
        };
        if last.timeline_frame.saturating_sub(current) > TIMELINE_CHUNK_TICKS * 2 {
            return false;
        }

        let config = match self.configs().ok().and_then(|configs| {
            configs
                .into_iter()
                .find(|config| config.models > result.observed_model)
        }) {
            Some(mut config) => {
                // idx不明の検索設定には仮のidxを置き、実際に観測したモデルは
                // 結果行から参照する。
                config.target_model = result.observed_model;
                config
            }
            None => return false,
        };
        let Some(result) = self.results.get_mut(self.selected_result) else {
            return false;
        };
        let Some(mut cursor) = result.timeline_cursor.take() else {
            return false;
        };
        if cursor.tick >= MAX_FUTURE_TIMELINE_TICKS || cursor.consumption >= MAX_SFMT_FRAME {
            result.timeline_cursor = Some(cursor);
            return false;
        }
        let chunk_end = cursor
            .tick
            .saturating_add(TIMELINE_CHUNK_TICKS)
            .min(MAX_FUTURE_TIMELINE_TICKS);
        let sfmt = self.timeline_sfmt.as_mut().expect("checked above");
        let mut events = Vec::new();
        while cursor.tick < chunk_end {
            let (used, blink) = cursor.status.next_state(sfmt);
            cursor.stream_cursor = cursor.stream_cursor.saturating_add(used as i64);
            cursor.consumption = cursor.consumption.saturating_add(used as i64);
            cursor.tick = cursor.tick.saturating_add(1);
            if blink.contains(&(config.target_model as u8)) {
                events.push((cursor.tick.saturating_mul(2), cursor.consumption));
            }
            if cursor.consumption >= MAX_SFMT_FRAME {
                break;
            }
        }
        let appended = events.len();
        for (wait_frame, consumption) in events {
            if let Some(previous) = result.timeline.last_mut() {
                previous.duration_frames = wait_frame.saturating_sub(previous.timeline_frame);
            }
            result.timeline.push(FieldTimelineRow {
                frame: consumption,
                timeline_frame: wait_frame,
                duration_frames: 0,
                observed: false,
            });
        }
        if let Some(target_wait) = result.target_wait_frame {
            let target_absolute = result.last_wait_frame.saturating_add(target_wait);
            result.target_blink_count = Some(
                result
                    .timeline
                    .iter()
                    .filter(|row| {
                        row.timeline_frame > result.last_wait_frame
                            && row.timeline_frame <= target_absolute
                    })
                    .count(),
            );
        }
        result.timeline_cursor = Some(cursor);
        appended > 0
    }

    /// UIスレッドでTimeline延長を開始する前の軽量な判定。
    /// 実際のModelStatus/SFMT走査は呼び出し側のワーカーで行う。
    pub(crate) fn timeline_extension_needed(&self, fps: f64) -> bool {
        if self.results.len() != 1 {
            return false;
        }
        let Some(current) = self.current_timeline_frame(fps) else {
            return false;
        };
        let Some(result) = self.results.get(self.selected_result) else {
            return false;
        };
        let Some(last) = result.timeline.last() else {
            return false;
        };
        last.timeline_frame.saturating_sub(current) <= TIMELINE_CHUNK_TICKS * 2
    }

    /// バックグラウンドで延長した状態のうち、Timeline関連だけを反映する。
    /// 入力欄・観測列・タイマー状態はUI側の最新値を保持する。
    pub(crate) fn apply_timeline_extension(&mut self, extended: FieldState) {
        if self.results.len() != extended.results.len()
            || self.selected_result != extended.selected_result
        {
            return;
        }
        self.results = extended.results;
        self.nearest_timeline_frame = extended.nearest_timeline_frame;
        self.timeline_sfmt = extended.timeline_sfmt;
        if self.current_timeline_sfmt.is_none() {
            self.current_timeline_sfmt = extended.current_timeline_sfmt;
            self.current_timeline_cursor = extended.current_timeline_cursor;
        }
    }

    pub fn observe(&mut self, fps: f64) {
        if !self.is_observation_input_enabled() {
            return;
        }
        if self.observed.len() >= crate::application::state::MAX_BLINK_OBSERVATIONS {
            self.set_actionable_status(format!(
                "瞬き観測は{}回までです。",
                crate::application::state::MAX_BLINK_OBSERVATIONS
            ));
            return;
        }
        let now = Instant::now();
        self.observed.push(now);
        if let Some(previous) = self.observed.iter().rev().nth(1) {
            self.intervals.push(
                seconds_to_game_frames(now.duration_since(*previous).as_secs_f64(), fps).round()
                    as i64,
            );
        }
        self.set_info_status(format!("瞬き{}回を記録しました。", self.observed.len()));
    }

    /// 一致済み候補を観測した間隔1つ分だけ延長する。
    ///
    /// 候補には最後に一致した瞬き直後の状態が保存されているため、次の瞬きまで
    /// 進めればよく、Shift入力ごとにSFMT検索全体をやり直さずに済む。
    pub fn advance_candidates_by_interval(&mut self, expected: i64) -> usize {
        let tolerance = self.tolerance.trim().parse::<i64>().ok().unwrap_or(-1);
        if tolerance < 0 || self.results.is_empty() {
            return 0;
        }
        let mut survivors = Vec::with_capacity(self.results.len());
        for mut result in self.results.drain(..) {
            let previous_last = result.last_wait_frame;
            let Some(next_index) = result
                .timeline
                .iter()
                .position(|row| row.timeline_frame > previous_last && !row.observed)
            else {
                continue;
            };
            let (next_wait, next_consumption) = {
                let row = &mut result.timeline[next_index];
                (row.timeline_frame, row.frame)
            };
            let actual = next_wait.saturating_sub(previous_last);
            if (actual - expected).abs() > tolerance {
                continue;
            }

            let target_absolute = result
                .target_wait_frame
                .map(|wait| previous_last.saturating_add(wait));
            result.timeline[next_index].observed = true;
            result.last_wait_frame = next_wait;
            result.last_consumption = next_consumption;
            result.intervals.push(actual);
            result.next_blink_wait_frame = result
                .timeline
                .iter()
                .find(|row| row.timeline_frame > next_wait)
                .map(|row| row.timeline_frame - next_wait)
                .unwrap_or_default();
            result.target_wait_frame = target_absolute
                .and_then(|absolute| (absolute >= next_wait).then_some(absolute - next_wait));
            if let Some(absolute) = target_absolute {
                result.target_blink_count = Some(
                    result
                        .timeline
                        .iter()
                        .filter(|row| {
                            row.timeline_frame > next_wait && row.timeline_frame <= absolute
                        })
                        .count(),
                );
            }
            survivors.push(result);
        }
        self.results = survivors;
        self.selected_result = 0;
        self.table_last_followed_row = None;
        self.refresh_nearest_timeline_frame();
        self.results.len()
    }

    #[allow(dead_code)]
    pub fn config(&self) -> Result<FieldConfig, String> {
        self.configs()?
            .into_iter()
            .next()
            .ok_or_else(|| "NPC範囲に観測対象idxを含む設定がありません。".to_string())
    }

    /// UIのNPC欄を単一または範囲として展開した検索設定。
    /// 主人公1体は各設定の`models`へ暗黙に加算する。
    pub fn configs(&self) -> Result<Vec<FieldConfig>, String> {
        let seed = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16)
            .map_err(|_| "初期SFMT SEEDは16進数で入力してください。".to_string())?;
        let range_start = parse_nonnegative(&self.range_start, "検索範囲の始点")?;
        let range_end = parse_nonnegative(&self.range_end, "検索範囲の終点")?;
        let npc_counts = parse_npc_counts(&self.npc_count, self.multiple_npc_search)?;
        let target_model = if self.target_mode == FieldTargetMode::Unknown {
            0
        } else {
            usize::try_from(parse_nonnegative(&self.target_model, "NPC番号")?)
                .map_err(|_| "NPC番号が大きすぎます。".to_string())?
        };
        let tolerance = parse_nonnegative(&self.tolerance, "許容差")?;
        let target_consumption = if self.target_consumption.trim().is_empty() {
            None
        } else {
            Some(parse_nonnegative(&self.target_consumption, "目標消費数")?)
        };

        let mut configs = Vec::with_capacity(npc_counts.len());
        for npc_count in npc_counts {
            let models = npc_count
                .checked_add(1)
                .ok_or_else(|| "NPC数が大きすぎます。".to_string())?;
            if models > MAX_MODELS {
                return Err(format!(
                    "NPC数が大きすぎます。主人公を含むモデル数は{}以下にしてください。",
                    MAX_MODELS
                ));
            }
            // idx不明は後段で全モデルを照合する。既知idxがNPC範囲の下限外にある
            // 設定は、そのidxに対して無効なモデル数として扱う。
            if self.target_mode == FieldTargetMode::Known && target_model >= models {
                continue;
            }
            configs.push(FieldConfig {
                seed,
                range_start,
                range_end,
                models,
                target_model,
                observed_intervals: self.intervals.clone(),
                tolerance,
                target_consumption,
            });
        }
        if configs.is_empty() {
            return Err(format!(
                "観測対象idxはNPC範囲内の0～{}で入力してください。",
                MAX_MODELS - 1
            ));
        }
        for config in &configs {
            validate_static_config(config)?;
        }
        Ok(configs)
    }

    pub fn start_blink_timer(&mut self, fps: f64) -> Result<(), String> {
        if self.mode == FieldMode::Correction {
            return Err(defaults::CORRECTION_TIMER_LOCKED.into());
        }
        if self.results.len() != 1 {
            return Err("候補が一意になるまでタイマーは開始しません。".into());
        }
        self.rebuild_blink_timer_queue(fps);
        self.start_blink_timer_from_queue(fps, true)
    }

    /// 明示的なずれ検証操作でのみ呼び出す。押下時の現在位置を保存して
    /// Timeline表を停止し、実測Frame・idx入力を解放する。
    pub fn enter_correction_mode(&mut self, fps: f64) {
        self.correction_timeline_frame = self.current_timeline_frame(fps);
        self.mode = FieldMode::Correction;
        self.target_input_locked = true;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.inferred_indices.clear();
        self.timer_start = None;
        self.timer_finished = false;
        self.clear_status();
    }

    fn rebuild_blink_timer_queue(&mut self, fps: f64) {
        let result = self.results.get(self.selected_result).cloned();

        let Some(result) = result else {
            self.blink_wait_queue.clear();
            return;
        };
        let current = self
            .current_timeline_frame(fps)
            .unwrap_or(result.last_wait_frame);
        let mut queue = result
            .timeline
            .iter()
            .filter(|row| row.timeline_frame > current)
            .map(|row| row.timeline_frame - result.last_wait_frame)
            .take(BLINK_TIMER_PREFETCH_COUNT)
            .collect::<VecDeque<_>>();
        if queue.is_empty() && result.next_blink_wait_frame > 0 {
            queue.push_back(result.next_blink_wait_frame);
        }
        self.blink_wait_queue = queue;
    }

    /// 先読みキューが空になる前に、既に生成済みのTimeline行を補充する。
    /// Timeline延長ワーカーが完了した次のUI更新でも同じ処理が走るため、
    /// 末尾到達を待ってから全キューを作り直すことはない。
    pub fn maintain_blink_timer_queue(&mut self, fps: f64) {
        if self.results.len() != 1 || self.mode == FieldMode::Correction {
            return;
        }
        // Targetタイマー中も次の瞬き表示を途切れさせないよう、消化済みの
        // 区間を捨てて先読みキューを補充する。NextBlink中は完了処理側が
        // 先頭を1件だけ取り除くため、ここでは追加だけを行う。
        if self.timer_kind == FieldTimerKind::Target {
            while self
                .blink_wait_queue
                .front()
                .is_some_and(|wait| self.remaining_from_observation(*wait, fps).is_err())
            {
                self.blink_wait_queue.pop_front();
            }
        }
        if self.blink_wait_queue.is_empty() {
            self.rebuild_blink_timer_queue(fps);
            return;
        }
        if self.blink_wait_queue.len() > BLINK_TIMER_REFILL_THRESHOLD {
            return;
        }
        let Some(result) = self.results.get(self.selected_result) else {
            return;
        };
        let last_queued_absolute = result
            .last_wait_frame
            .saturating_add(self.blink_wait_queue.back().copied().unwrap_or_default());
        let remaining = BLINK_TIMER_PREFETCH_COUNT.saturating_sub(self.blink_wait_queue.len());
        let additions = result
            .timeline
            .iter()
            .filter(|row| row.timeline_frame > last_queued_absolute)
            .map(|row| row.timeline_frame - result.last_wait_frame)
            .take(remaining)
            .collect::<Vec<_>>();
        self.blink_wait_queue.extend(additions);
    }

    /// Targetタイマー右上に表示する次の瞬きまでの残り時間。
    ///
    /// Target区間では瞬きタイマー自体を進めないため、表示専用の先読み
    /// キューから「まだ未来にある最初の区間」を選ぶ。区間の境界で古い
    /// 行が0秒になっても、次の行へ直ちに切り替わるので空表示にならない。
    pub fn preview_next_blink_remaining(&self, fps: f64) -> Option<f64> {
        for wait_frame in &self.blink_wait_queue {
            if let Ok(seconds) = self.remaining_from_observation(*wait_frame, fps) {
                return Some(seconds);
            }
        }
        let result = self.results.get(self.selected_result)?;
        let current = self.current_timeline_frame(fps)?;
        let wait_frame = result
            .timeline
            .iter()
            .find(|row| row.timeline_frame > current)
            .map(|row| row.timeline_frame - result.last_wait_frame)?;
        self.remaining_from_observation(wait_frame, fps).ok()
    }

    fn start_blink_timer_from_queue(&mut self, fps: f64, announce: bool) -> Result<(), String> {
        let wait_frame = self
            .blink_wait_queue
            .front()
            .copied()
            .ok_or_else(|| "次の瞬きを計算中です。".to_string())?;
        self.timer_seconds = self.remaining_from_observation(wait_frame, fps)?;
        self.timer_kind = FieldTimerKind::NextBlink;
        self.timer_start = Some(Instant::now());
        self.timer_finished = false;
        if announce {
            self.set_info_status(format!(
                "瞬きタイマー開始：次の瞬きまで{}Fです。",
                wait_frame
            ));
        }
        Ok(())
    }

    /// 現在のタイマー区間が終わった後、先読み済みの次の区間へ切り替える。
    /// Timelineがまだ延長中なら、キューを再構築してから一度だけ再試行する。
    pub fn advance_blink_timer(&mut self, fps: f64) -> Result<(), String> {
        if self.timer_kind != FieldTimerKind::NextBlink || self.results.len() != 1 {
            return Err("瞬きタイマーを継続できません。".into());
        }
        self.blink_wait_queue.pop_front();
        self.maintain_blink_timer_queue(fps);
        match self.start_blink_timer_from_queue(fps, false) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.rebuild_blink_timer_queue(fps);
                self.start_blink_timer_from_queue(fps, false)
            }
        }
    }

    pub fn start_target_timer(&mut self, fps: f64) -> Result<(), String> {
        if self.mode == FieldMode::Correction {
            return Err(defaults::CORRECTION_TIMER_LOCKED.into());
        }
        if self.results.len() != 1 {
            return Err("候補が一意になるまでタイマーは開始しません。".into());
        }
        let result = self
            .results
            .get(self.selected_result)
            .ok_or_else(|| "タイマーを開始する候補を選択してください。".to_string())?;
        let wait_frame = self
            .target_wait_frame_with_offset(result)
            .ok_or_else(|| "Target消費数に到達できない候補です。".to_string())?;
        self.timer_kind = FieldTimerKind::Target;
        match self.remaining_from_observation(wait_frame, fps) {
            Ok(seconds) => {
                self.timer_seconds = seconds;
                self.timer_start = Some(Instant::now());
                self.timer_finished = false;
                self.set_info_status(format!("タイマー開始：Targetまで{}Fです。", wait_frame));
            }
            Err(_) => {
                // Target情報は残っているためTargetモードへは切り替えられる。
                // ただし既に通過しているので、0秒で固定して通知音は鳴らさない。
                self.timer_seconds = 0.0;
                self.timer_start = None;
                self.timer_finished = true;
                self.set_actionable_status("TargetFrameを通り過ぎています。");
            }
        }
        Ok(())
    }

    /// 表示オフセット更新後、実行中のTargetタイマーの基準時刻を取り直す。
    /// 瞬きタイマーと保持中のTimelineは変更しない。
    pub fn refresh_target_timer_for_offset(&mut self, fps: f64) {
        if self.timer_kind != FieldTimerKind::Target || self.timer_start.is_none() {
            return;
        }
        let Some(result) = self.results.get(self.selected_result) else {
            return;
        };
        let Some(wait_frame) = self.target_wait_frame_with_offset(result) else {
            return;
        };
        match self.remaining_from_observation(wait_frame, fps) {
            Ok(seconds) => {
                self.timer_seconds = seconds;
                self.timer_start = Some(Instant::now());
                self.timer_finished = false;
            }
            Err(_) => {
                self.timer_seconds = 0.0;
                self.timer_start = None;
                self.timer_finished = true;
            }
        }
    }

    pub fn toggle_timer_kind(&mut self, fps: f64) -> Result<(), String> {
        if self.mode == FieldMode::Correction {
            return Err(defaults::CORRECTION_TIMER_LOCKED.into());
        }
        let previous = self.timer_kind;
        let next = match previous {
            FieldTimerKind::NextBlink => FieldTimerKind::Target,
            FieldTimerKind::Target => FieldTimerKind::NextBlink,
        };
        if next == FieldTimerKind::Target && !self.target_timer_available(fps) {
            return Err("TargetFrameのタイマーはまだ利用できません。".into());
        }
        self.timer_kind = next;
        if self.timer_start.is_some() {
            let result = match self.timer_kind {
                FieldTimerKind::NextBlink => self.start_blink_timer(fps),
                FieldTimerKind::Target => self.start_target_timer(fps),
            };
            if result.is_err() {
                self.timer_kind = previous;
            }
            result
        } else {
            // 完了したTargetタイマーから戻る場合は表示だけでなく瞬きカウントダウンを
            // 再開する。表示だけを切り替えると`timer_start`がなく通知音を予約できない。
            self.timer_finished = false;
            if self.timer_kind == FieldTimerKind::NextBlink {
                match self.start_blink_timer(fps) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.timer_kind = previous;
                        Err(error)
                    }
                }
            } else {
                Ok(())
            }
        }
    }

    pub fn target_timer_available(&self, _fps: f64) -> bool {
        if self.results.len() != 1 {
            return false;
        }
        self.results
            .get(self.selected_result)
            .is_some_and(|result| result.target_wait_frame.is_some())
    }

    /// Targetまでの表示Frameへ、フィールド専用オフセットを適用する。
    /// 候補のTarget到達判定・瞬き列・観測アンカーは変更しない。
    pub fn target_wait_frame_with_offset(&self, result: &FieldResult) -> Option<i64> {
        let wait = result.target_wait_frame?;
        if !self.use_offset {
            return Some(wait);
        }
        let trimmed = self.encounter_offset.trim();
        let offset = if trimmed.is_empty() {
            0
        } else {
            trimmed.parse::<i64>().ok()?
        };
        if !(-MAX_SFMT_FRAME..=MAX_SFMT_FRAME).contains(&offset) || offset % 2 != 0 {
            return None;
        }
        Some(wait.saturating_add(offset))
    }

    pub fn target_is_passed(&self, fps: f64) -> bool {
        let Some(result) = self.results.get(self.selected_result) else {
            return false;
        };
        let Some(wait_frame) = self.target_wait_frame_with_offset(result) else {
            return result.target_exact == Some(false)
                && result.target_frame_before.is_some()
                && result.target_frame_after.is_some();
        };
        self.remaining_from_observation(wait_frame, fps).is_err()
    }

    /// Target情報が消えた／無効になった場合は、表示を必ず瞬き側へ戻す。
    pub fn normalize_timer_kind(&mut self, fps: f64) {
        if self.timer_kind == FieldTimerKind::Target && !self.target_timer_available(fps) {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.timer_start.is_none() && self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            }
        }
    }

    pub fn timer_remaining(&self) -> Option<f64> {
        self.timer_start
            .map(|start| (self.timer_seconds - start.elapsed().as_secs_f64()).max(0.0))
    }

    pub fn adjust_by(&mut self, delta: i64, fps: f64) {
        self.adjust = self.adjust.saturating_add(delta);
        let shift = game_frames_to_seconds(delta.unsigned_abs() as f64, fps);
        if let Some(start) = self.timer_start {
            let shifted = if delta.is_positive() {
                start.checked_sub(std::time::Duration::from_secs_f64(shift))
            } else {
                start.checked_add(std::time::Duration::from_secs_f64(shift))
            };
            self.timer_start = shifted.or(Some(start));
        }
    }

    pub fn preview_remaining(&self, kind: FieldTimerKind, fps: f64) -> Option<f64> {
        if kind == self.timer_kind {
            if let Some(remaining) = self.timer_remaining() {
                return Some(remaining);
            }
            if self.timer_finished && kind == FieldTimerKind::Target {
                return Some(0.0);
            }
        }
        let result = self.results.get(self.selected_result)?;
        let wait_frame = match kind {
            FieldTimerKind::NextBlink => {
                let current = self.current_timeline_frame(fps)?;
                result
                    .timeline
                    .iter()
                    .find(|row| row.timeline_frame > current)
                    .map(|row| row.timeline_frame - result.last_wait_frame)?
            }
            FieldTimerKind::Target => self.target_wait_frame_with_offset(result)?,
        };
        self.remaining_from_observation(wait_frame, fps).ok()
    }

    fn remaining_from_observation(&self, wait_frame: i64, fps: f64) -> Result<f64, String> {
        let anchor = self
            .observed
            .last()
            .ok_or_else(|| "最後の観測瞬き時刻がありません。".to_string())?;
        let total = game_frames_to_seconds(wait_frame as f64, fps)
            - game_frames_to_seconds(self.adjust as f64, fps);
        let remaining = total - anchor.elapsed().as_secs_f64();
        if remaining <= 0.0 {
            return Err("指定したタイマー位置はすでに通過しています。再観測してください。".into());
        }
        Ok(remaining)
    }

    pub fn current_timeline_frame(&self, fps: f64) -> Option<i64> {
        let result = self.results.get(self.selected_result)?;
        if self.mode == FieldMode::Correction {
            if let Some(frame) = self.correction_timeline_frame {
                return Some(frame);
            }
        }
        let anchor = self.observed.last()?;
        Some(
            result.last_wait_frame
                + seconds_to_game_frames(anchor.elapsed().as_secs_f64(), fps).floor() as i64,
        )
    }

    pub fn current_timeline_row(&self, fps: f64) -> Option<usize> {
        let result = self.results.get(self.selected_result)?;
        let current = self.current_timeline_frame(fps)?;
        result
            .timeline
            .iter()
            .rposition(|row| row.timeline_frame <= current)
            .or(Some(0))
    }
}

pub(super) fn parse_nonnegative(text: &str, label: &str) -> Result<i64, String> {
    let value = text
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("{}は整数で入力してください。", label))?;
    if value < 0 {
        return Err(format!("{}は0以上で入力してください。", label));
    }
    Ok(value)
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn searches_all_start_consumptions() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 50_000,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let results = search(&config).unwrap();
        assert!(results.len() > 1);
        // 1つのSFMT開始位置から複数の観測窓が生じるため、順序は一意ではなく
        // 非減少となる。
        assert!(results
            .windows(2)
            .all(|rows| { rows[0].start_consumption <= rows[1].start_consumption }));
        assert!(results.iter().all(|result| {
            result
                .timeline
                .iter()
                .any(|row| row.frame == result.last_consumption && row.observed)
        }));
    }

    #[test]
    fn observed_intervals_can_start_at_a_later_blink() {
        let values = SearchValues::new(0, generate_u64_stream(1, 10_000));
        let mut status = ModelStatus::new(1);
        let mut stream_cursor = 0;
        let mut events = Vec::new();
        for tick in 0..MAX_WAIT_TICKS {
            let Some((_used, blink)) =
                next_state_with_values(&mut status, &mut stream_cursor, &values)
            else {
                break;
            };
            if blink.contains(&0) {
                events.push((tick + 1) * 2);
            }
        }
        let window_len = 4;
        let selected = (1..events.len().saturating_sub(window_len - 1))
            .find(|&start| {
                let expected = events[start + 1..start + window_len]
                    .windows(2)
                    .map(|pair| pair[1] - pair[0])
                    .collect::<Vec<_>>();
                !events[..start].windows(window_len).any(|window| {
                    window
                        .windows(2)
                        .map(|pair| pair[1] - pair[0])
                        .eq(expected.iter().copied())
                })
            })
            .expect("deterministic SFMT stream should provide a later unique window");
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10_000,
            models: 1,
            target_model: 0,
            observed_intervals: events[selected..selected + window_len]
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect(),
            tolerance: 0,
            target_consumption: None,
        };
        let result = search_candidate(0, &config, &values).expect("later blink window matches");
        assert!(selected > 0);
        assert_eq!(result.first_wait_frame, events[selected]);
    }

    #[test]
    fn targeted_search_refines_interval_matches_after_chunk_scan() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: Some(5_000),
        };
        let results = search(&config).unwrap();
        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|result| result.target_wait_frame.is_some()
                    || result.target_exact == Some(false))
        );
    }

    #[test]
    fn search_values_are_anchored_at_range_start() {
        let full = crate::domain::rng::generate_u64_values(1, 32);
        let anchored = generate_search_values(&FieldConfig {
            seed: 1,
            range_start: 10,
            range_end: 12,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 0,
            target_consumption: None,
        });

        assert_eq!(anchored.base, 10);
        assert_eq!(&anchored.values[..5], &full[10..15]);
    }

    #[test]
    fn search_pool_can_be_prepared_before_blink_intervals_exist() {
        let config = FieldConfig {
            seed: 1,
            range_start: 10,
            range_end: 12,
            models: 2,
            target_model: 0,
            observed_intervals: Vec::new(),
            tolerance: 5,
            target_consumption: Some(20),
        };
        let pool = prepare_search_pool(&config).unwrap();
        assert_eq!(pool.cache.read_values(10, 1).unwrap().len(), 1);
        assert!(pool.covers(&config));
    }

    #[test]
    fn cancelled_pool_generation_returns_without_building_results() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: Vec::new(),
            tolerance: 5,
            target_consumption: None,
        };
        let token = AtomicBool::new(true);
        assert!(matches!(
            prepare_search_pool_with_configs_cancelable(&[config], Some(&token)),
            Err(message) if message == SEARCH_CANCELLED
        ));
    }

    #[test]
    fn rejects_invalid_target_model() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 1_000,
            models: 1,
            target_model: 1,
            observed_intervals: vec![1],
            tolerance: 0,
            target_consumption: None,
        };
        assert!(search(&config).is_err());
    }

    #[test]
    fn ui_idx_controls_observed_model() {
        let mut state = FieldState {
            npc_count: "2".into(),
            target_model: "0".into(),
            ..FieldState::default()
        };
        assert_eq!(state.config().unwrap().models, 3);
        assert_eq!(state.config().unwrap().target_model, 0);
        state.target_model = "1".into();
        assert_eq!(state.config().unwrap().target_model, 1);
    }

    #[test]
    fn timeline_table_follows_current_position_by_default() {
        assert!(FieldState::default().table_follow_current);
    }

    #[test]
    fn session_phase_locks_timer_after_entering_correction() {
        let mut state = FieldState::default();
        assert_eq!(state.session_phase(), SessionPhase::Input);
        assert!(state.can_edit_observation_settings());
        state.mode = FieldMode::Idle;
        state.results.push(FieldResult {
            start_consumption: 0,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 0,
            last_consumption: 0,
            intervals: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 1,
            timeline: Vec::new(),
            target_blink_count: None,
            timeline_cursor: None,
            target_cursor: None,
        });
        assert_eq!(state.session_phase(), SessionPhase::TimelineReady);
        assert!(!state.can_edit_observation_settings());
        state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
        assert_eq!(state.session_phase(), SessionPhase::Correction);
        assert!(!state.can_edit_general_settings());
        assert!(state.can_edit_correction_inputs());
        assert!(state
            .start_blink_timer(crate::domain::timing::DEFAULT_FPS)
            .is_err());
    }

    #[test]
    fn npc_count_adds_the_implicit_player_model() {
        let state = FieldState {
            npc_count: "0".into(),
            ..FieldState::default()
        };
        assert_eq!(state.config().unwrap().models, 1);
    }

    #[test]
    fn multiple_npc_search_expands_the_requested_range() {
        let state = FieldState {
            npc_count: "10~20".into(),
            multiple_npc_search: true,
            ..FieldState::default()
        };
        let configs = state.configs().unwrap();
        assert_eq!(configs.len(), 11);
        assert_eq!(configs.first().unwrap().models, 11);
        assert_eq!(configs.last().unwrap().models, 21);
    }

    #[test]
    fn multiple_npc_search_rejects_ranges_wider_than_twenty_npc() {
        let state = FieldState {
            npc_count: "10~31".into(),
            multiple_npc_search: true,
            ..FieldState::default()
        };
        assert!(state.configs().is_err());
    }

    #[test]
    fn multiple_npc_search_allows_a_width_of_twenty() {
        let state = FieldState {
            npc_count: "10~30".into(),
            multiple_npc_search: true,
            ..FieldState::default()
        };
        assert_eq!(state.configs().unwrap().len(), 21);
    }

    #[test]
    fn npc_count_is_limited_to_zero_through_fifty() {
        let mut state = FieldState {
            npc_count: MAX_NPC_COUNT.to_string(),
            ..FieldState::default()
        };
        assert!(state.configs().is_ok());
        state.npc_count = (MAX_NPC_COUNT + 1).to_string();
        assert!(state.configs().is_err());
    }

    #[test]
    fn search_rejects_ranges_wider_than_one_hundred_thousand_frames() {
        let state = FieldState {
            range_start: "9000000".into(),
            range_end: "9200000".into(),
            ..FieldState::default()
        };
        assert!(state.config().is_err());
    }

    #[test]
    fn range_end_can_be_set_from_start_plus_width() {
        let mut state = FieldState {
            range_start: "9000000".into(),
            range_end: "0".into(),
            range_after_start: "10000".into(),
            ..FieldState::default()
        };
        state.set_range_from_start();
        assert_eq!(state.range_end, "9010000");
    }

    #[test]
    fn unique_timeline_extends_from_the_previous_remain_state() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: Some(100_000),
        };
        let mut results = search(&config).unwrap();
        let mut state = FieldState {
            range_start: "0".into(),
            range_end: "10".into(),
            npc_count: "0".into(),
            target_model: "0".into(),
            target_consumption: "100000".into(),
            intervals: vec![1],
            observed: vec![Instant::now() - std::time::Duration::from_secs(170)],
            results: vec![results.remove(0)],
            ..FieldState::default()
        };
        let before = state.results[0].timeline.len();
        assert!(state.extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS));
        assert!(state.results[0].timeline.len() > before);
        assert!(state.results[0].timeline.last().unwrap().timeline_frame > 10_000);
    }

    #[test]
    fn prepared_timeline_index_keeps_the_same_interval_candidates() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let baseline = search(&config).unwrap();
        let pool = prepare_search_pool_with_configs(std::slice::from_ref(&config)).unwrap();
        let prepared = search_with_preferred_idx_with_pool(&config, &pool)
            .unwrap()
            .results;
        assert_eq!(
            baseline
                .iter()
                .map(|result| result.start_consumption)
                .collect::<Vec<_>>(),
            prepared
                .iter()
                .map(|result| result.start_consumption)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unknown_target_search_keeps_idx_candidates_but_skips_target_check() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: Some(50),
        };
        let results = search_without_target_model(&config).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().any(|result| result.observed_model == 0));
        assert!(results.iter().any(|result| result.observed_model == 1));
        assert!(results
            .iter()
            .all(|result| result.target_wait_frame.is_none()));
    }

    #[test]
    fn observation_input_is_locked_after_unique_candidate() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Observing;
        assert!(state.is_observation_input_enabled());
        state.results.push(FieldResult {
            start_consumption: 477,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 478,
            last_consumption: 479,
            intervals: vec![2],
            next_blink_wait_frame: 2,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 1,
            timeline: Vec::new(),
            target_blink_count: None,
            timeline_cursor: None,
            target_cursor: None,
        });
        assert!(!state.is_observation_input_enabled());
        state.refresh_nearest_timeline_frame();
        assert_eq!(state.nearest_timeline_frame, Some(477));
    }

    #[test]
    fn target_remains_editable_until_explicit_correction_entry() {
        let mut state = FieldState::default();
        state.timer_start = Some(Instant::now());
        assert!(state.can_edit_target());

        state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
        assert!(!state.can_edit_target());

        state.cancel_observation();
        assert!(state.can_edit_target());
    }

    #[test]
    fn observation_input_is_capped_at_sixty_four_blinks() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Observing;
        state.observed = (0..crate::application::state::MAX_BLINK_OBSERVATIONS)
            .map(|_| Instant::now())
            .collect();
        state.observe(crate::domain::timing::DEFAULT_FPS);
        assert_eq!(
            state.observed.len(),
            crate::application::state::MAX_BLINK_OBSERVATIONS
        );
        assert_eq!(state.status_kind, StatusKind::Error);
    }

    #[test]
    fn zero_candidate_search_allows_additional_shift_for_retry() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Observing;
        state.search_exhausted = true;
        assert!(state.is_observation_input_enabled());
        state.start_observation();
        assert!(state.is_observation_input_enabled());
    }

    #[test]
    fn invalidating_search_results_keeps_observation_but_clears_runtime_state() {
        let mut state = FieldState::default();
        state.observed.push(Instant::now());
        state.intervals.push(200);
        state.results.push(FieldResult {
            start_consumption: 477,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 478,
            last_consumption: 479,
            intervals: vec![2],
            next_blink_wait_frame: 2,

            target_wait_frame: Some(4),
            target_exact: Some(true),
            target_frame_before: Some(10),
            target_frame_after: Some(10),
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            target_blink_count: Some(1),
            timeline_cursor: None,
            target_cursor: None,
        });
        state.timer_start = Some(Instant::now());
        state.target_dirty = true;

        state.invalidate_search_results("条件を変更しました。");

        assert!(state.results.is_empty());
        assert!(state.inferred_indices.is_empty());
        assert!(state.timer_start.is_none());
        assert_eq!(state.observed.len(), 1);
        assert_eq!(state.intervals, vec![200]);
        assert!(state.target_dirty);
        assert_eq!(state.status, "条件を変更しました。");
    }

    #[test]
    fn target_edit_search_results_are_accepted_while_cancelled_results_are_not() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Idle;
        state.intervals = vec![200, 210];
        assert!(state.accepts_search_result());

        state.cancel_observation();
        assert!(!state.accepts_search_result());
    }

    #[test]
    fn timer_kind_can_toggle_when_idle() {
        let mut state = FieldState::default();
        state.observed.push(Instant::now());
        state.results.push(FieldResult {
            start_consumption: 477,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 478,
            last_consumption: 479,
            intervals: vec![2],
            next_blink_wait_frame: 2,
            target_wait_frame: Some(4),
            target_exact: Some(true),
            target_frame_before: Some(10),
            target_frame_after: Some(10),
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            target_blink_count: Some(1),
            timeline_cursor: None,
            target_cursor: None,
        });
        assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
        state.toggle_timer_kind(59.8621).unwrap();
        assert_eq!(state.timer_kind, FieldTimerKind::Target);
        state.toggle_timer_kind(59.8621).unwrap();
        assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
    }

    #[test]
    fn field_offset_only_shifts_target_wait_frame() {
        let mut state = FieldState::default();
        state.use_offset = true;
        state.encounter_offset = "+2".into();
        let result = FieldResult {
            start_consumption: 0,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 1,
            last_consumption: 2,
            intervals: vec![2],
            next_blink_wait_frame: 2,
            target_wait_frame: Some(100),
            target_exact: Some(true),
            target_frame_before: Some(50),
            target_frame_after: Some(50),
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            target_blink_count: Some(1),
            timeline_cursor: None,
            target_cursor: None,
        };
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(102));
        state.encounter_offset = "-2".into();
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(98));
        state.encounter_offset = (MAX_SFMT_FRAME + 2).to_string();
        assert_eq!(state.target_wait_frame_with_offset(&result), None);
        state.encounter_offset.clear();
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(100));
        state.use_offset = false;
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(100));
    }

    #[test]
    fn completed_target_timer_stays_at_zero_until_reset() {
        let mut state = FieldState::default();
        state.timer_kind = FieldTimerKind::Target;
        state.timer_finished = true;
        assert_eq!(
            state.preview_remaining(FieldTimerKind::Target, 59.8621),
            Some(0.0)
        );
        state.cancel_observation();
        assert_eq!(
            state.preview_remaining(FieldTimerKind::Target, 59.8621),
            None
        );
    }

    #[test]
    fn target_timing_distinguishes_exact_and_crossing_steps() {
        let values = SearchValues::new(0, generate_u64_stream(1, 32));
        let event = BlinkEvent {
            wait_frame: 0,
            consumption: 0,
            status: ModelStatus::new(2),
            stream_cursor: 0,
        };
        let crossing = find_target_timing(&event, 1, &values).unwrap();
        assert!(!crossing.exact);
        assert_eq!(crossing.absolute_wait_frame, 2);
        assert_eq!((crossing.frame_before, crossing.frame_after), (0, 2));

        let exact = find_target_timing(&event, 2, &values).unwrap();
        assert!(exact.exact);
        assert_eq!(exact.absolute_wait_frame, 2);
    }
}
