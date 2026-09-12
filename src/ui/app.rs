use crate::application::defaults;
use crate::application::field::{FieldState, FieldTargetMode, FieldTimerKind};
use crate::application::state::{AppState, SavedOffset};
use crate::domain::timing::beep_offsets_for_segment;
use crate::infrastructure::audio::{fallback_beep, fallback_candidate_not_found, AudioOutput};
use crate::infrastructure::persistence::{
    PersistedAppState, PersistedField, PersistedHatch, PersistedNormalTimer,
};
use crate::ui::app_types::*;
use crate::ui::beep_scheduler::BeepScheduler;
use crate::ui::field_view::field_ui;
use crate::ui::hatch_view::hatch_guidance_ui;
use crate::ui::i18n::{Language, Texts};
use crate::ui::layout::{
    ACTION_ROW_HEIGHT, LEFT_PANEL_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, PANEL_GAP,
    PANEL_PADDING as PANEL_INNER_PADDING, TIMER_PANEL_HEIGHT as TIMER_DISPLAY_HEIGHT,
    TOP_PANEL_HEIGHT,
};
use crate::ui::search_jobs::SearchJobs;
use crate::ui::settings_view::settings_ui;
use crate::ui::timer_view::normal_timer_ui;
use crate::ui::widgets::{
    draw_timer_flash, fixed_group_panel, frame_spinner, horizontal_frame_spinner,
    install_app_style, install_japanese_font, nonnegative_frame_spinner_with_commit, npc_spinner,
    numeric_text_edit, offset_spinner_with_commit, wide_button, ColorScheme, FlashMode,
    COMPACT_ACTION_SIZE,
};
use eframe::egui;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const TIMELINE_HEADER_HEIGHT: f32 = 24.0;

pub struct UiApp {
    pub state: AppState,
    language: Language,
    color_scheme: ColorScheme,
    tab: AppTab,
    pub field: FieldState,
    jobs: SearchJobs,
    adjustment_input: String,
    audio: Option<AudioOutput>,
    correction_dialog: Option<CorrectionDialog>,
    correction_calculating: bool,
    shift_was_down: bool,
    beeps: BeepScheduler,
    /// Timeline延長が完了するまで、次の瞬きタイマー開始を再試行する。
    field_timer_retry: bool,
    /// beepと同時に左上のタイマー枠を強調表示する終了時刻。
    timer_flash_until: Option<Instant>,
    offset_manager: Option<OffsetManagerState>,
    beep_enabled: bool,
    keyboard_enabled: bool,
    candidate_search_sound: bool,
    flash_mode: FlashMode,
    wheel_offset_enabled: bool,
    /// 最後に保存した入力スナップショット。実行中の検索・タイマー状態は含めない。
    last_saved_state: Option<PersistedAppState>,
    save_deadline: Option<Instant>,
}

impl UiApp {
    // --- Construction and application lifecycle --------------------------

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_japanese_font(&cc.egui_ctx);
        let saved = PersistedAppState::load();
        let mut app = Self::default();
        app.apply_persisted_state(saved);
        app.last_saved_state = Some(app.persisted_state());
        // 音声デバイスの初期化を最初の開始操作へ遅延させると、
        // その瞬間だけUIが固まることがある。起動時に一度だけ準備し、
        // 孵化・フィールド・通常タイマーで共有する。
        if app.beep_enabled {
            app.audio = AudioOutput::open();
        }
        // Rayonのグローバルワーカープールも初回検索時ではなく、
        // 起動直後のバックグラウンドで準備する。最初のSpace/開始操作で
        // スレッド生成が重なってUIが一瞬止まるのを避ける。
        std::thread::spawn(|| {
            let _ = rayon::ThreadPoolBuilder::new().build_global();
        });
        install_app_style(&cc.egui_ctx, app.color_scheme);
        app
    }
}

impl Default for UiApp {
    fn default() -> Self {
        Self {
            state: AppState::default(),
            language: Language::default(),
            color_scheme: ColorScheme::default(),
            tab: AppTab::Hatch,
            field: FieldState::default(),
            jobs: SearchJobs::default(),
            adjustment_input: "0".into(),
            audio: None,
            correction_dialog: None,
            correction_calculating: false,
            shift_was_down: false,
            beeps: BeepScheduler::default(),
            field_timer_retry: false,
            timer_flash_until: None,
            offset_manager: None,
            beep_enabled: defaults::DEFAULT_BEEP_ENABLED,
            keyboard_enabled: defaults::DEFAULT_KEYBOARD_ENABLED,
            candidate_search_sound: defaults::DEFAULT_CANDIDATE_SEARCH_SOUND,
            flash_mode: FlashMode::default(),
            wheel_offset_enabled: defaults::DEFAULT_WHEEL_OFFSET_ENABLED,
            last_saved_state: None,
            save_deadline: None,
        }
    }
}

impl eframe::App for UiApp {
    // --- Frame update, input dispatch, and view composition ---------------

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persisted_state().save();
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let texts = Texts::new(self.language);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(texts.app_title().to_owned()));
        let hatch_was_unique = self.state.candidates.len() == 1;
        let field_was_observing = self.field.is_observing();
        self.poll_generation();
        self.poll_field_pool_generation(ctx);
        self.poll_field_timeline_generation();
        self.poll_field_timeline_extension();
        self.start_field_timeline_extension(ctx);
        self.poll_field_generation(ctx);
        self.poll_field_idx_inference(ctx);
        self.poll_guidance_generation();
        self.poll_candidate_reachability();
        if self.correction_calculating {
            let correction_finished = self
                .jobs
                .correction_generation
                .as_ref()
                .map(JoinHandle::is_finished)
                .unwrap_or(true);
            if correction_finished {
                let generation_key = self.jobs.correction_generation_key.take();
                let mut result_is_current =
                    self.state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
                if let Some(handle) = self.jobs.correction_generation.take() {
                    result_is_current &= generation_key.is_some_and(|(seed, start, end, fps)| {
                        self.state.search_config().ok().is_some_and(|config| {
                            config.seed == seed
                                && config.range_start == start
                                && config.range_end == end
                                && config.fps.to_bits() == fps
                        })
                    });
                    match handle.join() {
                        Ok(Ok(cache)) if result_is_current => {
                            self.state.apply_observation_cache(cache)
                        }
                        Ok(Err(message)) => {
                            if result_is_current {
                                self.state.status = message;
                            }
                        }
                        Err(_) => {
                            if result_is_current {
                                self.state.status = "タイムライン計算に失敗しました。".into();
                            }
                        }
                        Ok(Ok(_)) => {}
                    }
                }
                self.correction_calculating = false;
                if result_is_current {
                    self.state.prepare_correction_timeline();
                    self.open_correction_dialog_if_ready();
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        // リセット／キャンセルは描画中に発生するため、次のフレームの処理前に
        // 保留中の追従beepを消去する。
        if self.state.is_idle() {
            self.beeps.article.clear();
            self.beeps.article_start = None;
        }
        if self.field.is_observing()
            || (self.field.session_phase() == crate::application::state::SessionPhase::Input
                && self.field.results.is_empty()
                && !self.field.timer_is_running())
        {
            self.beeps.field.clear();
            self.beeps.field_start = None;
            self.field_timer_retry = false;
        }
        let was_production = self.state.is_production();
        let window_focused = ctx.input(|input| input.focused);
        let global_shortcuts_enabled =
            self.keyboard_enabled && window_focused && !ctx.wants_keyboard_input();
        let space = ctx.input(|input| input.key_pressed(egui::Key::Space));
        let shift = ctx.input(|input| input.modifiers.shift);
        let shift_pressed = shift && !self.shift_was_down;
        self.shift_was_down = shift;
        let enter = ctx.input(|input| input.key_pressed(egui::Key::Enter));
        let left = ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft));
        let right = ctx.input(|input| input.key_pressed(egui::Key::ArrowRight));
        let up = ctx.input(|input| input.key_pressed(egui::Key::ArrowUp));
        let down = ctx.input(|input| input.key_pressed(egui::Key::ArrowDown));
        // 補正ボタンと同じ有効状態で左右キーを受け付け、画面上の操作と
        // キーボード操作を同じ状態遷移へ接続する。
        if global_shortcuts_enabled && self.tab == AppTab::Hatch && self.state.is_blink_phase() {
            if left {
                self.state.adjust_by(-1);
                self.adjustment_input = self.state.adjust.to_string();
            }
            if right {
                self.state.adjust_by(1);
                self.adjustment_input = self.state.adjust.to_string();
            }
        }
        if global_shortcuts_enabled && self.tab == AppTab::Field && (left || right) {
            self.field
                .adjust_by(if right { 1 } else { -1 }, self.state.fps_value());
        }
        if global_shortcuts_enabled && space && self.tab == AppTab::Field {
            if self.field.is_observing() {
                self.field.cancel_observation();
                self.cancel_field_background_tasks();
            } else if self.field.session_phase() == crate::application::state::SessionPhase::Input
                && self.field.results.is_empty()
                && self.jobs.field_generation.is_none()
                && self.jobs.field_pool_generation.is_none()
                && self.jobs.field_idx_inference.is_none()
            {
                match self.field.configs() {
                    Ok(_) => self.field.start_observation(),
                    Err(message) => {
                        self.field.set_actionable_status(message);
                    }
                }
            }
        } else if global_shortcuts_enabled
            && shift_pressed
            && self.tab == AppTab::Field
            && self.field.is_observation_input_enabled()
            && self.jobs.field_idx_inference.is_none()
        {
            self.record_field_blink(ctx);
        } else if global_shortcuts_enabled && self.tab == AppTab::Hatch && space {
            if self.state.is_idle() {
                self.start_observation_and_generation(ctx);
            } else if self.state.is_observing() {
                self.state.cancel_observation();
            }
        } else if global_shortcuts_enabled && self.tab == AppTab::Hatch && shift_pressed {
            self.record_hatch_blink();
        } else if global_shortcuts_enabled && self.tab == AppTab::NormalTimer && space {
            self.toggle_normal_timer();
        }
        if self.tab == AppTab::Field
            && field_was_observing
            && self.field.is_observing()
            && self.jobs.field_generation.is_none()
            && self.jobs.field_pool_generation.is_none()
        {
            let reusable = self
                .field
                .configs()
                .ok()
                .and_then(|configs| {
                    self.jobs
                        .field_pool
                        .as_ref()
                        .map(|pool| configs.iter().all(|config| pool.covers(config)))
                })
                .unwrap_or(false);
            if !reusable {
                self.jobs.field_pool = None;
                self.start_field_pool_generation(ctx);
            }
        }
        if !hatch_was_unique && self.state.candidates.len() == 1 && self.state.is_ready() {
            self.play_candidate_confirmed();
        }
        if enter
            && global_shortcuts_enabled
            && self.tab == AppTab::Field
            && !self.field.is_correction_mode()
            && self.field.results.len() == 1
            && self.jobs.field_generation.is_none()
            && self.jobs.field_pool_generation.is_none()
            && self.jobs.field_idx_inference.is_none()
        {
            self.field.enter_correction_mode(self.state.fps_value());
            self.beeps.field.clear();
            self.beeps.field_start = None;
        }
        // タイムライン仕様のタイマーモードは、テキスト入力欄にフォーカスがないときだけ
        // 上下キーで切り替える。入力欄のカーソル操作とは競合させない。
        if self.tab == AppTab::Field && global_shortcuts_enabled && (up || down) {
            if let Err(message) = self.field.toggle_timer_kind(self.state.fps_value()) {
                self.field.set_actionable_status(message);
            } else {
                // モード切替では新しいタイマー区間を作る。Target区間の終了音を
                // 瞬きタイマーへ持ち越さない。
                self.beeps.field.clear();
                self.beeps.field_start = None;
            }
        }
        if self.state.is_countdown() {
            // 各段階の終了時刻からbeepを逆算する。まず現在段階の予約を
            // 処理してから段階遷移を行うことで、前段階のbeepを次段階へ
            // 持ち越したり、次段階と重ねたりしない。
            self.ensure_article_beep_schedule();
            let due = BeepScheduler::drain_due(&mut self.beeps.article, Instant::now());
            for _ in 0..due {
                self.play_beep();
            }
            if self.state.tick_article_timer().is_some() {
                let due = BeepScheduler::drain_due(&mut self.beeps.article, Instant::now());
                for _ in 0..due {
                    self.play_beep();
                }
                self.beeps.article.clear();
                self.beeps.article_start = None;
                self.ensure_article_beep_schedule();
            }
            let due = BeepScheduler::drain_due(&mut self.beeps.article, Instant::now());
            for _ in 0..due {
                self.play_beep();
            }
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        if self.tab == AppTab::Hatch && self.jobs.guidance_generation.is_none() {
            self.start_guidance_generation(ctx);
        }
        self.maybe_start_candidate_reachability(ctx);
        if !self.beeps.article.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if !was_production && self.state.is_production() {
            self.beeps.article = self.schedule_segment_beeps(
                self.state.timer_start_time(),
                self.state.timer_seconds_value(),
            );
            self.beeps.article_start = self.state.timer_start_time();
        }

        // タイマーは表示中タブに依存せず進める。別タブを見ている間も
        // 瞬き／Target到達・beep・区間遷移を失わないようにする。
        // 停止・モード切替時に不要な予約を残さない。
        if self.field_timer_retry && !self.field.timer_is_running() && self.field.results.len() == 1
        {
            if self.field.start_blink_timer(self.state.fps_value()).is_ok() {
                self.field_timer_retry = false;
            } else {
                // Timeline延長のワーカーがまだ終わっていない。短い間隔で
                // 再試行し、タイマーと通知音を停止状態のまま放置しない。
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        if !self.field.timer_is_running() {
            self.beeps.field.clear();
            self.beeps.field_start = None;
        }
        self.field
            .maintain_blink_timer_queue(self.state.fps_value());
        self.ensure_field_beep_schedule();
        let due = BeepScheduler::drain_due(&mut self.beeps.field, Instant::now());
        for _ in 0..due {
            self.play_beep();
        }
        if self
            .field
            .timer_remaining()
            .is_some_and(|remaining| remaining <= 0.0)
            && !(self.field.timer_kind() == FieldTimerKind::Target
                && self.field.timer_is_finished())
        {
            let completed_kind = self.field.timer_kind();
            match completed_kind {
                FieldTimerKind::NextBlink => {
                    if let Err(message) = self
                        .field
                        .finish_blink_timer_segment(self.state.fps_value())
                    {
                        // ちょうどTimelineの末尾で延長処理が走っていると、
                        // 次の行が一時的にまだ見えないことがある。ここで
                        // タイマーを終了扱いにせず、延長後に再試行する。
                        self.field_timer_retry = true;
                        self.field
                            .set_info_status(format!("次の瞬きを計算中… ({message})"));
                    }
                }
                FieldTimerKind::Target => {
                    // タイマーとTimelineは0秒でも保持する。瞬きカウントダウンへの切替や
                    // 補正モードへの移行は明示操作でのみ行い、その操作だけが停止条件になる。
                    self.field.finish_target_timer();
                }
            }
        }
        let due = BeepScheduler::drain_due(&mut self.beeps.field, Instant::now());
        for _ in 0..due {
            self.play_beep();
        }
        if self.field.timer_is_running()
            || !self.beeps.field.is_empty()
            || self.field.results.len() == 1
        {
            // フィールドのTimeline Currentはゲーム内1/30秒単位で進む。候補がある間は
            // 同じ周期で再描画し、表示行が内部時計から遅れないようにする。
            ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 30.0));
        }

        if let Some(deadline) = self.state.next_blink_deadline() {
            if Instant::now() >= deadline {
                self.play_beep();
                self.state.advance_predicted_blink();
            }
            // 1F補正は約16.7msなので、50ms周期では補正より大きな発音遅延が出る。
            ctx.request_repaint_after(Duration::from_millis(5));
        }

        if self.state.is_production() {
            let due = BeepScheduler::drain_due(&mut self.beeps.article, Instant::now());
            for _ in 0..due {
                self.play_beep();
            }
            if self.state.remaining() <= 0.0 {
                let due = BeepScheduler::drain_due(&mut self.beeps.article, Instant::now());
                for _ in 0..due {
                    self.play_beep();
                }
                self.state.finish_production();
                self.state.status = "Target Frameに到達しました。終了です。".into();
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }

        self.tick_normal_timer(ctx);

        let now = Instant::now();
        let timer_flashing = self
            .timer_flash_until
            .is_some_and(|deadline| deadline > now);
        if timer_flashing {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            self.timer_flash_until = None;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.tab == AppTab::Hatch, texts.hatch_tab())
                    .clicked()
                {
                    self.tab = AppTab::Hatch;
                }
                if ui
                    .selectable_label(self.tab == AppTab::Field, texts.field_tab())
                    .clicked()
                {
                    self.tab = AppTab::Field;
                }
                if ui
                    .selectable_label(self.tab == AppTab::NormalTimer, texts.normal_timer_tab())
                    .clicked()
                {
                    self.tab = AppTab::NormalTimer;
                }
                if ui
                    .selectable_label(self.tab == AppTab::Settings, texts.settings_tab())
                    .clicked()
                {
                    self.tab = AppTab::Settings;
                }
            });
            ui.separator();
            if self.tab == AppTab::Settings {
                let (_, settings_rect) =
                    ui.allocate_space(egui::vec2(ui.available_width(), ui.available_height()));
                let mut preferences_changed = false;
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(settings_rect), |ui| {
                    preferences_changed = settings_ui(
                        ui,
                        &mut self.state,
                        &mut self.language,
                        &mut self.color_scheme,
                        &mut self.beep_enabled,
                        &mut self.keyboard_enabled,
                        &mut self.candidate_search_sound,
                        &mut self.flash_mode,
                        &mut self.wheel_offset_enabled,
                        ctx,
                        &texts,
                    );
                });
                if preferences_changed
                    && !self.state.is_timer_phase()
                    && !self.state.candidates.is_empty()
                {
                    self.state.recalculate_production();
                }
                return;
            }
            if self.tab == AppTab::Field {
                let field_actions = field_ui(
                    ui,
                    &mut self.field,
                    &mut self.jobs.field_generation,
                    &mut self.jobs.field_pool_generation,
                    &mut self.jobs.field_idx_inference,
                    ctx,
                    self.state.fps_value(),
                    timer_flashing,
                    self.wheel_offset_enabled,
                    self.color_scheme,
                    &texts,
                );
                if field_actions.cancelled {
                    self.cancel_field_background_tasks();
                }
                if field_actions.observe_requested {
                    self.record_field_blink(ctx);
                }
                if field_actions.correction_requested {
                    self.field.enter_correction_mode(self.state.fps_value());
                    self.beeps.field.clear();
                    self.beeps.field_start = None;
                }
                if field_actions.target_offset_changed {
                    self.field
                        .refresh_target_timer_for_offset(self.state.fps_value());
                }
                if field_actions.open_offset_manager {
                    self.open_offset_manager(OffsetScope::Field);
                }
                if field_actions.idx_inference_requested {
                    self.start_field_idx_inference(ctx);
                }
                if field_actions.idx_inference_cancel_requested {
                    self.cancel_field_idx_inference();
                }
                if field_actions.adjust_changed {
                    self.beeps.field.clear();
                    self.beeps.field_start = None;
                }
                // 瞬きタイマー実行中もTargetだけは再評価できる。Seed/NPCは固定する一方、
                // 確定したTargetから派生表示を更新する必要がある。
                let target_refinement_requested =
                    field_actions.target_committed && self.field.target_research_pending;
                let observation_search_requested =
                    field_actions.search_requested && self.field.intervals.len() >= 2;
                if (target_refinement_requested || observation_search_requested)
                    && !self.field.target_dirty
                {
                    if self.jobs.field_generation.is_none()
                        && (target_refinement_requested || self.field.can_edit_general_settings())
                    {
                        self.start_field_search(ctx);
                    } else if target_refinement_requested {
                        // 先行検索が終わるまでTargetだけを編集可能にしているため、
                        // その検索が終わった直後に現在のTargetで再評価する。
                        self.jobs.field_search_needs_restart = true;
                    }
                }
                if !field_was_observing
                    && self.field.is_observing()
                    && self.jobs.field_generation.is_none()
                {
                    if let Some(token) = self.jobs.field_pool_generation_cancel.take() {
                        token.store(true, Ordering::Release);
                    }
                    self.jobs.field_pool_generation = None;
                    self.jobs.field_pool = None;
                    self.start_field_pool_generation(ctx);
                }
                return;
            }
            if self.tab == AppTab::NormalTimer {
                let fps = self.state.fps_value();
                let (toggle, open_manager) = normal_timer_ui(
                    ui,
                    &mut self.state.normal_timer,
                    fps,
                    timer_flashing,
                    self.color_scheme,
                    self.wheel_offset_enabled,
                    &texts,
                );
                if open_manager {
                    self.open_offset_manager(OffsetScope::Normal);
                }
                if toggle {
                    self.toggle_normal_timer();
                }
                return;
            }

            // 最小ウィンドウでは表を数行だけ見せる。表の全行数で親レイアウトが
            // 高くならないよう、上段の高さを固定し、拡大時だけ少し追従させる。
            let text_scale = (ui.available_width() / 1000.0).clamp(1.0, 1.6);
            let top_height = (TOP_PANEL_HEIGHT * text_scale)
                .min(300.0)
                .min(ui.available_height() * 0.55);
            let (_, top_rect) = ui.allocate_space(egui::vec2(ui.available_width(), top_height));
            let mut top_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(top_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let right_panel_width = (top_rect.width() - LEFT_PANEL_WIDTH - PANEL_GAP).max(0.0);
            let left_rect =
                egui::Rect::from_min_size(top_rect.min, egui::vec2(LEFT_PANEL_WIDTH, top_height));
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(left_rect.right() + PANEL_GAP, top_rect.top()),
                egui::vec2(right_panel_width, top_height),
            );
            top_ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    let panel_rect = ui.max_rect();
                    let padding = PANEL_INNER_PADDING;
                    let blink_timer = !self.state.is_timer_phase();
                    let show_blink_prediction = blink_timer
                        && self.state.candidates.len() == 1
                        && self.state.has_next_blink();

                    // モード別の案内はタイマー枠の右上に置く。○×欄の
                    // `next >>` は別の意味（○の猶予時間）なので、そこで
                    // 上書きしない。
                    let timer_side_label = self
                        .state
                        .hatch_guidance()
                        .map(|guidance| {
                            if guidance.search_out_of_range {
                                texts.search_out_of_range().to_string()
                            } else if self.state.fastest_close {
                                guidance
                                    .next_circle_seconds
                                    .map(|seconds| texts.next_circle_seconds(seconds))
                                    .unwrap_or_else(|| "—".into())
                            } else {
                                guidance
                                    .target_wait_seconds
                                    .map(|seconds| texts.production_wait_seconds(seconds))
                                    .or_else(|| {
                                        guidance
                                            .target_unreachable
                                            .then(|| texts.production_wait_unknown().to_string())
                                    })
                                    .unwrap_or_else(|| "—".into())
                            }
                        })
                        .unwrap_or_else(|| "—".into());

                    // 上段左枠は固定矩形の中へ、Current・タイマー・補正を絶対配置する。
                    let current_rect = egui::Rect::from_min_size(
                        panel_rect.min + egui::vec2(padding, padding),
                        egui::vec2(panel_rect.width() - padding * 2.0, TIMELINE_HEADER_HEIGHT),
                    );
                    let current_width = current_rect.width() * 0.5;
                    let current_left_rect = egui::Rect::from_min_size(
                        current_rect.min,
                        egui::vec2(current_width, current_rect.height()),
                    );
                    let current_right_rect = egui::Rect::from_min_size(
                        egui::pos2(current_rect.left() + current_width, current_rect.top()),
                        egui::vec2(current_rect.width() - current_width, current_rect.height()),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(current_left_rect), |ui| {
                        ui.label(
                            texts.current_frame_optional(self.state.timeline_display_position()),
                        );
                    });
                    ui.allocate_new_ui(
                        egui::UiBuilder::new()
                            .max_rect(current_right_rect)
                            .layout(egui::Layout::right_to_left(egui::Align::Center)),
                        |ui| {
                            ui.label(
                                egui::RichText::new(timer_side_label.clone())
                                    .size(14.0 * text_scale)
                                    .color(self.color_scheme.colors_for_ui(ui).timer_text),
                            );
                        },
                    );

                    let timer_rect = egui::Rect::from_center_size(
                        egui::pos2(panel_rect.center().x, panel_rect.center().y - 4.0),
                        egui::vec2(
                            panel_rect.width() - padding * 2.0,
                            TIMER_DISPLAY_HEIGHT * text_scale,
                        ),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(timer_rect), |ui| {
                        ui.with_layout(
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                if self.state.is_countdown() {
                                    ui.label(
                                        egui::RichText::new(
                                            texts.seconds(self.state.article_remaining()),
                                        )
                                        .size(48.0 * text_scale)
                                        .strong()
                                        .color(self.color_scheme.colors_for_ui(ui).timer_text),
                                    );
                                } else if self.state.is_encounter() {
                                    ui.label(
                                        egui::RichText::new(texts.b_input().to_owned())
                                            .size(48.0 * text_scale)
                                            .strong()
                                            .color(self.color_scheme.colors_for_ui(ui).timer_text),
                                    );
                                } else if self.state.is_production() {
                                    ui.label(
                                        egui::RichText::new(texts.seconds(self.state.remaining()))
                                            .size(48.0 * text_scale)
                                            .strong()
                                            .color(self.color_scheme.colors_for_ui(ui).timer_text),
                                    );
                                } else if show_blink_prediction {
                                    let seconds = self
                                        .state
                                        .next_blink_deadline()
                                        .map(|deadline| {
                                            deadline
                                                .saturating_duration_since(Instant::now())
                                                .as_secs_f64()
                                        })
                                        .unwrap_or(0.0);
                                    ui.label(
                                        egui::RichText::new(texts.seconds(seconds))
                                            .size(48.0 * text_scale)
                                            .strong()
                                            .color(self.color_scheme.colors_for_ui(ui).timer_text),
                                    );
                                } else if self.state.is_idle() || self.state.is_observing() {
                                    ui.label(
                                        egui::RichText::new(texts.seconds(0.0))
                                            .size(48.0 * text_scale)
                                            .strong()
                                            .color(self.color_scheme.colors_for_ui(ui).timer_text),
                                    );
                                }
                            },
                        );
                    });

                    let correction_rect = egui::Rect::from_min_size(
                        egui::pos2(
                            panel_rect.left() + padding,
                            panel_rect.bottom() - 4.0 - 32.0,
                        ),
                        egui::vec2(panel_rect.width() - padding * 2.0, ACTION_ROW_HEIGHT),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(correction_rect), |ui| {
                        correction_ui(
                            ui,
                            &mut self.state,
                            &mut self.adjustment_input,
                            self.wheel_offset_enabled,
                            &texts,
                        );
                    });
                },
            );
            draw_timer_flash(&top_ui, left_rect, timer_flashing, self.color_scheme);
            top_ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    hatch_guidance_ui(
                        ui,
                        &mut self.state,
                        top_height,
                        text_scale,
                        self.color_scheme,
                        &texts,
                    );
                },
            );

            ui.separator();
            // 下段も左右の枠を先に固定し、各コンポーネントは枠内で相対配置する。
            // ウィンドウ下端まで枠を伸ばし、下段の外側に余白を残さない。
            // 補正操作は実測Frame欄の隣に置き、下段はその行に必要な高さだけを確保する。
            let bottom_height = ui.available_height().max(0.0);
            let (_, bottom_rect) =
                ui.allocate_space(egui::vec2(ui.available_width(), bottom_height));
            let right_panel_width = (bottom_rect.width() - LEFT_PANEL_WIDTH - PANEL_GAP).max(0.0);
            let bottom_left_rect = egui::Rect::from_min_size(
                bottom_rect.min,
                egui::vec2(LEFT_PANEL_WIDTH, bottom_rect.height()),
            );
            let bottom_right_rect = egui::Rect::from_min_size(
                egui::pos2(bottom_left_rect.right() + PANEL_GAP, bottom_rect.top()),
                egui::vec2(right_panel_width, bottom_rect.height()),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bottom_left_rect), |ui| {
                fixed_group_panel(ui, |ui| {
                    let (start_requested, finish_requested, observe_requested) =
                        observation_ui(ui, &mut self.state, self.color_scheme, &texts);
                    if start_requested {
                        self.start_observation_and_generation(ctx);
                    }
                    if observe_requested {
                        self.record_hatch_blink();
                    }
                    if finish_requested {
                        self.state.finish_fastest_close_for_correction();
                    }
                });
            });
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bottom_right_rect), |ui| {
                fixed_group_panel(ui, |ui| {
                    let (changed, correct_offset, open_manager, observed_talk_changed) = input_ui(
                        ui,
                        &mut self.state,
                        self.wheel_offset_enabled,
                        self.correction_calculating,
                        &texts,
                    );
                    if changed {
                        if observed_talk_changed || self.state.can_edit_correction_inputs() {
                            self.state.recalculate_production_preserving_guidance();
                        } else {
                            self.state.recalculate_production();
                        }
                        self.state.status.clear();
                    }
                    if open_manager {
                        self.open_offset_manager(OffsetScope::Hatch);
                    }
                    if correct_offset {
                        self.request_correction(ctx);
                    }
                });
            });
        });

        // Target入力欄のEnter確定処理を先に終えてから、本番移行を行う。描画前に
        // 遷移すると入力中のTargetが未確定のまま判定されるため、孵化側のEnterだけ
        // 描画後に処理する。
        if enter
            && self.keyboard_enabled
            && window_focused
            && self.tab == AppTab::Hatch
            && self.state.is_ready()
        {
            self.state.target_dirty = false;
            if self.state.fastest_close {
                self.state.finish_fastest_close_for_correction();
            } else {
                self.beeps.article.clear();
                self.beeps.article_start = None;
                self.state.begin_encounter();
            }
        }

        self.show_correction_dialog(ctx, &texts);

        self.show_offset_manager(ctx, &texts);

        self.persist_if_changed(ctx);

        if self.jobs.generation.is_some()
            || self.jobs.field_generation.is_some()
            || self.jobs.field_pool_generation.is_some()
            || self.jobs.field_timeline_generation.is_some()
            || self.jobs.field_timeline_extension.is_some()
            || self.jobs.field_idx_inference.is_some()
            || self.jobs.guidance_generation.is_some()
            || self.jobs.candidate_reachability.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl UiApp {
    // --- Persisted input snapshot conversion ------------------------------

    fn persisted_state(&self) -> PersistedAppState {
        PersistedAppState {
            language: self.language,
            fps: self.state.fps.clone(),
            color_scheme: self.color_scheme,
            beep_enabled: self.beep_enabled,
            keyboard_enabled: self.keyboard_enabled,
            candidate_search_sound: self.candidate_search_sound,
            flash_mode: self.flash_mode,
            wheel_offset_enabled: self.wheel_offset_enabled,
            tab: match self.tab {
                AppTab::Hatch => 0,
                AppTab::Field => 1,
                AppTab::NormalTimer => 2,
                AppTab::Settings => 3,
            },
            hatch: PersistedHatch {
                seed: self.state.seed.clone(),
                range_start: self.state.range_start.clone(),
                range_end: self.state.range_end.clone(),
                range_before_target: self.state.range_before_target.clone(),
                target: self.state.target.clone(),
                actual_target_frame: self.state.actual_target_frame.clone(),
                npc: self.state.npc.clone(),
                blank_frames: self.state.blank_frames.clone(),
                encounter_offset: self.state.encounter_offset.clone(),
                use_offset: self.state.use_offset,
                rotom_threshold: self.state.rotom_threshold.clone(),
                consider_rotom_talk: self.state.consider_rotom_talk,
                consider_pre_rotom_consumption: self.state.consider_pre_rotom_consumption,
                pre_rotom_consumption: self.state.pre_rotom_consumption.clone(),
                consider_npc_initial_load: self.state.consider_npc_initial_load,
                npc_initial_load: self.state.npc_initial_load.clone(),
                fastest_close: self.state.fastest_close,
                tolerance: self.state.tolerance.clone(),
                adjust: self.state.adjust,
            },
            field: PersistedField {
                seed: self.field.seed.clone(),
                range_start: self.field.range_start.clone(),
                range_end: self.field.range_end.clone(),
                range_after_start: self.field.range_after_start.clone(),
                npc_count: self.field.npc_count.clone(),
                multiple_npc_search: self.field.multiple_npc_search,
                target_model: self.field.target_model.clone(),
                target_mode_known: self.field.target_mode == FieldTargetMode::Known,
                actual_frame: self.field.actual_frame.clone(),
                tolerance: self.field.tolerance.clone(),
                target_consumption: self.field.target_consumption.clone(),
                encounter_offset: self.field.encounter_offset.clone(),
                use_offset: self.field.use_offset,
                table_follow_current: self.field.table_follow_current,
                adjust: self.field.adjust,
            },
            normal_timer: PersistedNormalTimer {
                wait_frames: self.state.normal_timer.wait_frames.clone(),
                offsets: self.state.normal_timer.offsets.clone(),
                offset_enabled: self.state.normal_timer.offset_enabled.clone(),
            },
            offsets: self.state.offset_store.clone(),
        }
    }

    fn apply_persisted_state(&mut self, saved: PersistedAppState) {
        self.language = saved.language;
        self.color_scheme = saved.color_scheme;
        self.beep_enabled = saved.beep_enabled;
        self.keyboard_enabled = saved.keyboard_enabled;
        self.candidate_search_sound = saved.candidate_search_sound;
        self.flash_mode = saved.flash_mode;
        self.wheel_offset_enabled = saved.wheel_offset_enabled;
        self.state.fps = saved.fps;
        self.state.offset_store = saved.offsets;

        let hatch = saved.hatch;
        self.state.seed = hatch.seed;
        self.state.range_start = hatch.range_start;
        self.state.range_end = hatch.range_end;
        self.state.range_before_target = hatch.range_before_target;
        self.state.target = hatch.target;
        self.state.actual_target_frame = hatch.actual_target_frame;
        self.state.npc = hatch.npc;
        self.state.blank_frames = hatch.blank_frames;
        self.state.encounter_offset = hatch.encounter_offset;
        self.state.use_offset = hatch.use_offset;
        self.state.rotom_threshold = hatch.rotom_threshold;
        self.state.consider_rotom_talk = hatch.consider_rotom_talk;
        self.state.consider_pre_rotom_consumption = hatch.consider_pre_rotom_consumption;
        self.state.pre_rotom_consumption = hatch.pre_rotom_consumption;
        self.state.consider_npc_initial_load = hatch.consider_npc_initial_load;
        self.state.npc_initial_load = hatch.npc_initial_load;
        self.state.fastest_close = hatch.fastest_close;
        self.state.tolerance = hatch.tolerance;
        self.state.adjust = hatch.adjust;
        self.adjustment_input = hatch.adjust.to_string();

        let field = saved.field;
        self.field.seed = field.seed;
        self.field.range_start = field.range_start;
        self.field.range_end = field.range_end;
        self.field.range_after_start = field.range_after_start;
        self.field.npc_count = field.npc_count;
        self.field.multiple_npc_search = field.multiple_npc_search;
        self.field.target_model = field.target_model;
        // フィールドはNPC番号を明示する入力モードだけを使用する。
        // 保存データを読み込む際も、この指定番号の経路へ正規化する。
        self.field.target_mode = FieldTargetMode::Known;
        self.field.actual_frame = field.actual_frame;
        self.field.tolerance = field.tolerance;
        self.field.target_consumption = field.target_consumption;
        self.field.encounter_offset = field.encounter_offset;
        self.field.use_offset = field.use_offset;
        self.field.table_follow_current = field.table_follow_current;
        self.field.adjust = field.adjust;

        let normal = saved.normal_timer;
        self.state.normal_timer.wait_frames = normal.wait_frames;
        self.state.normal_timer.offsets = normal.offsets;
        self.state.normal_timer.offset_enabled = normal.offset_enabled;
        while self.state.normal_timer.offset_enabled.len() < self.state.normal_timer.offsets.len() {
            self.state.normal_timer.offset_enabled.push(true);
        }
        self.tab = match saved.tab {
            1 => AppTab::Field,
            2 => AppTab::NormalTimer,
            3 | 4 => AppTab::Settings,
            _ => AppTab::Hatch,
        };
    }

    fn persist_if_changed(&mut self, ctx: &egui::Context) {
        let current = self.persisted_state();
        if self.last_saved_state.as_ref() != Some(&current) {
            self.save_deadline
                .get_or_insert_with(|| Instant::now() + Duration::from_millis(300));
            ctx.request_repaint_after(Duration::from_millis(320));
        }
        if self
            .save_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            current.clone().save();
            self.last_saved_state = Some(current);
            self.save_deadline = None;
        }
    }

    fn start_guidance_generation(&mut self, ctx: &egui::Context) {
        let Some(request) = self.state.guidance_search_request() else {
            return;
        };
        let repaint = ctx.clone();
        self.jobs.guidance_generation = Some(std::thread::spawn(move || {
            let results = AppState::calculate_guidance_batch(request);
            repaint.request_repaint();
            (request, results)
        }));
    }

    fn poll_guidance_generation(&mut self) {
        let finished = self
            .jobs
            .guidance_generation
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let handle = self
            .jobs
            .guidance_generation
            .take()
            .expect("guidance handle");
        let (request, results) = match handle.join() {
            Ok(result) => result,
            Err(_) => return,
        };
        // 設定を編集したら到達判定の対応表を消去し、別のSeed・Target・オフセットへ
        // 古い結果を適用しない。
        if self
            .state
            .guidance_search_request()
            .is_some_and(|current| current.same_configuration(request))
        {
            self.state.apply_guidance_results(request, results);
        }
    }

    fn maybe_start_candidate_reachability(&mut self, ctx: &egui::Context) {
        if self.jobs.candidate_reachability.is_some() {
            return;
        }
        let Some(request) = self.state.candidate_reachability_request() else {
            self.jobs.candidate_reachability_key = None;
            return;
        };
        if self
            .jobs
            .candidate_reachability_key
            .as_ref()
            .is_some_and(|key| key == &request)
        {
            return;
        }
        self.jobs.candidate_reachability_key = Some(request.clone());
        let repaint = ctx.clone();
        self.jobs.candidate_reachability = Some(std::thread::spawn(move || {
            let results = AppState::calculate_candidate_reachability(request.clone());
            repaint.request_repaint();
            (request, results)
        }));
    }

    fn poll_candidate_reachability(&mut self) {
        let finished = self
            .jobs
            .candidate_reachability
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let handle = self
            .jobs
            .candidate_reachability
            .take()
            .expect("candidate reachability handle");
        let Ok((request, results)) = handle.join() else {
            return;
        };
        self.state.apply_candidate_reachability(&request, &results);
    }

    fn start_correction_timeline_calculation(&mut self, ctx: &egui::Context) {
        // 観測間隔は残っているがキャッシュがまだない場合は、補正操作から
        // 観測Timeline生成も開始する。生成中は補正ボタンをスピナーへ置き換え、
        // 完了後に候補照合・本番Timeline計算へ続ける。
        if self.state.candidates.is_empty()
            && self.state.observation_cache.is_none()
            && self.state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS
            && self.jobs.correction_generation.is_none()
        {
            let Ok(config) = self.state.search_config() else {
                return;
            };
            self.jobs.correction_generation_key = Some((
                config.seed,
                config.range_start,
                config.range_end,
                config.fps.to_bits(),
            ));
            let repaint = ctx.clone();
            self.jobs.correction_generation = Some(std::thread::spawn(move || {
                let cache = crate::domain::rng::generate_observation_cache(
                    config.seed,
                    config.range_start..=config.range_end,
                    config.target,
                    config.fps,
                );
                repaint.request_repaint();
                cache
            }));
        }
    }

    /// ずれ検証の入口を一つに集約する。ボタンとEnterのどちらからでも
    /// 同じ条件確認・バックグラウンド計算・ダイアログ表示を行う。
    fn request_correction(&mut self, ctx: &egui::Context) {
        let already_in_correction =
            self.state.session_phase() == crate::application::state::SessionPhase::Correction;
        // 最速閉じは実測Frame入力前にReadyからFinishedへ移る。通常タイマーも
        // 終了時にFinishedを使うため、両方の状態から補正を開始できる。
        if (!already_in_correction && !self.state.can_enter_correction())
            || !self.state.correction_inputs_complete()
        {
            return;
        }
        if !already_in_correction {
            self.state.enter_correction_mode();
        }
        if self.state.correction_timeline_needs_calculation() {
            self.state.status.clear();
            self.correction_calculating = true;
            self.start_correction_timeline_calculation(ctx);
            ctx.request_repaint();
        } else {
            self.open_correction_dialog_if_ready();
        }
    }

    fn open_correction_dialog_if_ready(&mut self) {
        // 開始位置は補正欄のドロップダウンで選択する。計算ボタンを押すたびに
        // その時点の選択値を読み込み、古い候補や最初の候補を再利用しない。
        let Some(frame) = self.state.selected_correction_start() else {
            self.state.status.clear();
            return;
        };
        match self.state.correction_preview_from(frame) {
            Ok(preview) => self.correction_dialog = Some(CorrectionDialog::Confirm { preview }),
            Err(_) => self.state.status.clear(),
        }
    }

    fn apply_correction_offset(&mut self, offset: i64) {
        self.state.use_offset = true;
        self.state.encounter_offset = offset.to_string();
        self.state.recalculate_production();
        self.state.status = format!("オフセットを{}Fに変更しました。", offset);
    }

    fn apply_correction_rotom(&mut self, correction: crate::application::state::RotomCorrection) {
        self.state.rotom_threshold = correction.suggested_threshold.to_string();
        self.state.observed_rotom_talk = Some(correction.observed_talk);
        self.state.recalculate_production();
    }

    fn play_beep(&mut self) {
        self.timer_flash_until =
            Some(Instant::now() + Duration::from_millis(self.flash_mode.duration_millis()));
        if !self.beep_enabled {
            return;
        }
        if self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        if !self.audio.as_ref().is_some_and(AudioOutput::beep) {
            fallback_candidate_not_found();
        }
    }

    fn show_correction_dialog(&mut self, ctx: &egui::Context, texts: &Texts) {
        let Some(mut dialog) = self.correction_dialog.take() else {
            return;
        };
        let mut keep = true;
        let title = match &dialog {
            CorrectionDialog::ConfirmFieldCorrection { .. } => texts.correction_confirmation(),
            CorrectionDialog::Confirm { .. } => texts.correction_confirmation(),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            // 保存されたウィンドウ位置に左右されず、補正ダイアログを画面中央に置く。
            // 最小幅はidxボタン列が狭い画面でも収まる寸法を確保する。
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .min_width(MIN_WINDOW_WIDTH)
            .min_height(MIN_WINDOW_HEIGHT)
            .show(ctx, |ui| match &mut dialog {
                CorrectionDialog::ConfirmFieldCorrection {
                    indices,
                    unknown,
                    suggested_offset,
                } => {
                    ui.label(texts.correction_calculated());
                    let current_offset = if self.field.use_offset {
                        self.field
                            .encounter_offset
                            .trim()
                            .parse::<i64>()
                            .unwrap_or_default()
                    } else {
                        0
                    };
                    if let Some(offset) = suggested_offset {
                        let offset_matches = current_offset == *offset;
                        ui.horizontal(|ui| {
                            ui.label(texts.offset());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if offset_matches {
                                        ui.label("☑");
                                    } else if ui.button(texts.set()).clicked() {
                                        self.field.use_offset = true;
                                        self.field.encounter_offset = offset.to_string();
                                    }
                                    ui.label(format!("{}F", offset));
                                },
                            );
                        });
                    }
                    ui.separator();
                    ui.label(texts.model_number());
                    ui.horizontal(|ui| {
                        if *unknown {
                            ui.label("?");
                        } else {
                            let current_idx = self.field.target_model.trim().parse::<usize>().ok();
                            for idx in indices.iter().copied() {
                                if current_idx == Some(idx) {
                                    ui.add_enabled(false, egui::Button::new(format!("{idx} ☑")));
                                } else if ui.button(texts.idx_set(idx)).clicked() {
                                    self.field.target_model = idx.to_string();
                                    self.field.target_research_pending = true;
                                    self.field.mark_timeline_ready();
                                    self.field.set_info_status(format!(
                                        "NPC番号 {}を入力しました。Timelineを保持して再検索中…",
                                        idx
                                    ));
                                    self.start_field_search(ctx);
                                    break;
                                }
                            }
                        }
                    });
                    if ui.button(texts.no()).clicked() {
                        keep = false;
                    }
                }
                CorrectionDialog::Confirm { preview } => {
                    ui.label(texts.correction_calculated());

                    let rotom = preview.rotom_correction;
                    let rotom_value = rotom
                        .map(|value| value.suggested_threshold)
                        .unwrap_or_else(|| self.state.rotom_threshold_value());
                    let rotom_matches = rotom.is_none_or(|value| {
                        self.state.rotom_threshold_value() == value.suggested_threshold
                    });
                    ui.horizontal(|ui| {
                        ui.label(texts.rotom_chatter_label());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if rotom_matches {
                                ui.label("☑");
                            } else if let Some(correction) = rotom {
                                if ui.button(texts.set()).clicked() {
                                    self.apply_correction_rotom(correction);
                                }
                            }
                            ui.label(format!("{}%", rotom_value));
                        });
                    });

                    let current_offset = if self.state.use_offset {
                        self.state
                            .encounter_offset
                            .trim()
                            .parse::<i64>()
                            .unwrap_or_default()
                    } else {
                        0
                    };
                    let offset_matches = current_offset == preview.suggested_offset;
                    ui.horizontal(|ui| {
                        ui.label(texts.offset());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if offset_matches {
                                ui.label("☑");
                            } else if ui.button(texts.set()).clicked() {
                                self.apply_correction_offset(preview.suggested_offset);
                            }
                            ui.label(format!("{}F", preview.suggested_offset));
                        });
                    });

                    ui.horizontal(|ui| {
                        ui.label(texts.timeline_reachability());
                        ui.label(format!("(Start:{}F)", preview.timeline_start));
                    });
                    ui.horizontal(|ui| {
                        ui.label(texts.target_actual_reachability(
                            preview.target_on_timeline,
                            preview.actual_on_timeline,
                        ));
                    });

                    if ui.button(texts.no()).clicked() {
                        keep = false;
                    }
                }
            });
        self.correction_dialog = keep.then_some(dialog);
    }

    fn play_candidate_confirmed(&mut self) {
        if !self.beep_enabled || !self.candidate_search_sound {
            return;
        }
        if self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        if !self
            .audio
            .as_ref()
            .is_some_and(AudioOutput::candidate_confirmed)
        {
            fallback_beep();
        }
    }

    fn play_candidate_not_found(&mut self) {
        if !self.beep_enabled || !self.candidate_search_sound {
            return;
        }
        if self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        if !self
            .audio
            .as_ref()
            .is_some_and(AudioOutput::candidate_not_found)
        {
            fallback_candidate_not_found();
        }
    }

    /// フィールド観測をリセットしたとき、検索ワーカーへキャンセルを通知して破棄する。
    /// 生成・照合中も共有トークンを確認するため、キャンセル後にCPUを使い続けたり、
    /// 長い検索の完了まで入力を無効にしたりしない。
    fn cancel_field_background_tasks(&mut self) {
        for token in [
            self.jobs.field_generation_cancel.as_ref(),
            self.jobs.field_pool_generation_cancel.as_ref(),
            self.jobs.field_timeline_generation_cancel.as_ref(),
            self.jobs.field_idx_inference_cancel.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            token.store(true, Ordering::Release);
        }
        self.jobs.field_generation.take();
        self.jobs.field_generation_cancel.take();
        self.jobs.field_search_needs_restart = false;
        self.jobs.field_pool_generation.take();
        self.jobs.field_pool_generation_cancel.take();
        self.jobs.field_timeline_generation.take();
        self.jobs.field_timeline_generation_cancel.take();
        self.jobs.field_timeline_extension.take();
        self.jobs.field_timeline_extension_key.take();
        self.jobs.field_timeline_extension_blocked = false;
        self.jobs.field_idx_inference.take();
        self.jobs.field_idx_inference_cancel.take();
        self.jobs.field_pool = None;
        self.beeps.field.clear();
        self.beeps.field_start = None;
    }

    fn toggle_normal_timer(&mut self) {
        if self.state.normal_timer.is_running() {
            self.state.normal_timer.stop();
            self.beeps.normal.clear();
            return;
        }

        if self
            .state
            .normal_timer
            .start(self.state.fps_value())
            .is_ok()
        {
            self.schedule_normal_timer_beeps();
        }
    }

    fn tick_normal_timer(&mut self, ctx: &egui::Context) {
        let due = BeepScheduler::drain_due(&mut self.beeps.normal, Instant::now());
        for _ in 0..due {
            self.play_beep();
        }

        while self.state.normal_timer.is_running() && self.state.normal_timer.remaining() <= 0.0 {
            let due = BeepScheduler::drain_due(&mut self.beeps.normal, Instant::now());
            for _ in 0..due {
                self.play_beep();
            }
            let advanced = self
                .state
                .normal_timer
                .advance_segment(self.state.fps_value());
            self.beeps.normal.clear();
            if !advanced {
                break;
            }
            self.schedule_normal_timer_beeps();
            let due = BeepScheduler::drain_due(&mut self.beeps.normal, Instant::now());
            for _ in 0..due {
                self.play_beep();
            }
        }

        if self.state.normal_timer.is_running() || !self.beeps.normal.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn normal_beep_parameters(&self) -> (u32, f64) {
        let count = self
            .state
            .beep_count
            .trim()
            .parse::<u32>()
            .unwrap_or(1)
            .max(1);
        let interval = self
            .state
            .beep_interval
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(1.0);
        (count, interval)
    }

    fn schedule_normal_timer_beeps(&mut self) {
        let Some(start) = self.state.normal_timer.timer_start else {
            self.beeps.normal.clear();
            return;
        };
        self.beeps.normal =
            self.schedule_segment_beeps(Some(start), self.state.normal_timer.timer_seconds);
    }

    fn ensure_article_beep_schedule(&mut self) {
        if !self.state.is_countdown() {
            return;
        }
        let Some(start) = self.state.article_timer_start_time() else {
            return;
        };
        if self.beeps.article_start == Some(start) {
            return;
        }
        self.beeps.article_start = Some(start);
        self.beeps.article =
            self.schedule_segment_beeps(Some(start), self.state.article_timer_seconds_value());
    }

    fn ensure_field_beep_schedule(&mut self) {
        let Some(start) = self.field.timer_start_time() else {
            return;
        };
        if self.beeps.field_start == Some(start) {
            return;
        }
        self.beeps.field_start = Some(start);
        // フィールド観測のbeepは予測瞬きそのものを示すため常に1回とする。
        // 設定した回数はTargetタイマーなど長いカウントダウンだけで使う。
        let (count, interval) = match self.field.timer_kind() {
            FieldTimerKind::NextBlink => (1, self.normal_beep_parameters().1),
            FieldTimerKind::Target => self.normal_beep_parameters(),
        };
        self.beeps.field = self.schedule_segment_beeps_with_parameters(
            Some(start),
            self.field.timer_seconds_value(),
            count,
            interval,
        );
    }

    fn schedule_segment_beeps(
        &self,
        start: Option<Instant>,
        duration_seconds: f64,
    ) -> Vec<Instant> {
        let (count, interval) = self.normal_beep_parameters();
        self.schedule_segment_beeps_with_parameters(start, duration_seconds, count, interval)
    }

    fn schedule_segment_beeps_with_parameters(
        &self,
        start: Option<Instant>,
        duration_seconds: f64,
        count: u32,
        interval: f64,
    ) -> Vec<Instant> {
        let Some(start) = start else {
            return Vec::new();
        };
        let duration_seconds = if duration_seconds.is_finite() {
            duration_seconds.max(0.0)
        } else {
            0.0
        };
        beep_offsets_for_segment(duration_seconds, count, interval)
            .into_iter()
            .map(|offset| start + Duration::from_secs_f64(offset))
            .collect()
    }

    fn open_offset_manager(&mut self, scope: OffsetScope) {
        self.offset_manager = Some(OffsetManagerState {
            scope,
            value: "0".into(),
            memo: String::new(),
            editing: None,
            status: String::new(),
        });
    }

    fn show_offset_manager(&mut self, ctx: &egui::Context, texts: &Texts) {
        let Some(mut manager) = self.offset_manager.take() else {
            return;
        };
        let wheel_offset_enabled = self.wheel_offset_enabled;
        let mut open = true;
        let title = match manager.scope {
            OffsetScope::Hatch => texts.hatch_offset_manager(),
            OffsetScope::Field => texts.field_offset_manager(),
            OffsetScope::Normal => texts.normal_offset_manager(),
        };
        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.offset_frame());
                    frame_spinner(ui, &mut manager.value, 100.0, 2, wheel_offset_enabled);
                });
                ui.horizontal(|ui| {
                    ui.label(texts.memo());
                    ui.add(egui::TextEdit::singleline(&mut manager.memo).desired_width(260.0));
                    let editing = manager.editing.is_some();
                    if ui
                        .button(if editing { texts.update() } else { texts.add() })
                        .clicked()
                    {
                        match manager.value.trim().parse::<i64>() {
                            Ok(value) => {
                                let entry = SavedOffset {
                                    value,
                                    memo: manager.memo.trim().to_string(),
                                };
                                let entries = match manager.scope {
                                    OffsetScope::Hatch => &mut self.state.offset_store.hatch,
                                    OffsetScope::Field => &mut self.state.offset_store.field,
                                    OffsetScope::Normal => &mut self.state.offset_store.normal,
                                };
                                if let Some(index) = manager.editing {
                                    if let Some(existing) = entries.get_mut(index) {
                                        *existing = entry;
                                    }
                                } else {
                                    entries.push(entry);
                                }
                                match self.state.offset_store.save() {
                                    Ok(()) => {
                                        manager.status.clear();
                                        manager.value = "0".into();
                                        manager.memo.clear();
                                        manager.editing = None;
                                    }
                                    Err(message) => manager.status = message,
                                }
                            }
                            Err(_) => {
                                manager.status = "オフセットFrameは整数で入力してください。".into()
                            }
                        }
                    }
                    if editing && ui.button(texts.stop_editing()).clicked() {
                        manager.editing = None;
                        manager.value = "0".into();
                        manager.memo.clear();
                    }
                });
                ui.separator();

                let entries = match manager.scope {
                    OffsetScope::Hatch => self.state.offset_store.hatch.clone(),
                    OffsetScope::Field => self.state.offset_store.field.clone(),
                    OffsetScope::Normal => self.state.offset_store.normal.clone(),
                };
                let mut action = None;
                for (index, entry) in entries.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new(format!("{}F", entry.value)));
                        ui.add_sized(
                            [260.0, 20.0],
                            egui::Label::new(if entry.memo.is_empty() {
                                texts.no_memo()
                            } else {
                                entry.memo.as_str()
                            }),
                        );
                        if ui.button(texts.apply()).clicked() {
                            action = Some(OffsetAction::Apply(entry.value));
                        }
                        if ui.button(texts.edit()).clicked() {
                            action = Some(OffsetAction::Edit(index));
                        }
                        if ui.button(texts.delete()).clicked() {
                            action = Some(OffsetAction::Delete(index));
                        }
                    });
                }

                if let Some(action) = action {
                    match action {
                        OffsetAction::Apply(value) => match manager.scope {
                            OffsetScope::Hatch => {
                                if self.state.is_timer_phase() || self.state.is_finished() {
                                    manager.status = "本番中はオフセットを反映できません。".into();
                                } else {
                                    self.state.use_offset = true;
                                    self.state.encounter_offset = value.to_string();
                                    self.state.recalculate_production();
                                    manager.status.clear();
                                }
                            }
                            OffsetScope::Field => {
                                if self.field.timer_is_running() {
                                    manager.status = "タイマー停止中のみ反映できます。".into();
                                } else {
                                    self.field.use_offset = true;
                                    self.field.encounter_offset = value.to_string();
                                    self.field
                                        .refresh_target_timer_for_offset(self.state.fps_value());
                                    manager.status.clear();
                                }
                            }
                            OffsetScope::Normal => {
                                if self.state.normal_timer.is_running() {
                                    manager.status = "タイマー停止中のみ反映できます。".into();
                                } else {
                                    self.state.normal_timer.offsets.push(value.to_string());
                                    self.state.normal_timer.offset_enabled.push(true);
                                    manager.status.clear();
                                }
                            }
                        },
                        OffsetAction::Edit(index) => {
                            if let Some(entry) = entries.get(index) {
                                manager.value = entry.value.to_string();
                                manager.memo = entry.memo.clone();
                                manager.editing = Some(index);
                            }
                        }
                        OffsetAction::Delete(index) => {
                            let entries = match manager.scope {
                                OffsetScope::Hatch => &mut self.state.offset_store.hatch,
                                OffsetScope::Field => &mut self.state.offset_store.field,
                                OffsetScope::Normal => &mut self.state.offset_store.normal,
                            };
                            if index < entries.len() {
                                entries.remove(index);
                                manager.editing = None;
                                match self.state.offset_store.save() {
                                    Ok(()) => manager.status.clear(),
                                    Err(message) => manager.status = message,
                                }
                            }
                        }
                    }
                }
                if !manager.status.is_empty() {
                    let status = texts.status(&manager.status);
                    ui.colored_label(self.color_scheme.colors_for_ui(ui).error, status);
                }
            });
        if open {
            self.offset_manager = Some(manager);
        }
    }

    fn record_hatch_blink(&mut self) {
        if !self.state.is_observing() {
            return;
        }
        let was_exhausted = self.state.search_exhausted;
        self.state.observe();
        if !was_exhausted && self.state.search_exhausted {
            self.play_candidate_not_found();
        }
    }

    fn record_field_blink(&mut self, ctx: &egui::Context) {
        if !self.field.is_observation_input_enabled() || self.jobs.field_idx_inference.is_some() {
            return;
        }
        if self.field.search_exhausted {
            return;
        }
        // 初回検索で候補が得られた後は、StartingFrameから全Timelineを作り直さず
        // 次の瞬き1回分だけ候補を延長する。初回検索中は再利用可能な状態がないため、
        // 再検索要求として扱う。
        let had_candidates = !self.field.results.is_empty();
        let search_running = self.jobs.field_generation.is_some();
        let previous_interval_count = self.field.intervals.len();
        self.field.observe(self.state.fps_value());
        if had_candidates && !search_running && previous_interval_count >= 2 {
            let expected = self.field.intervals.last().copied().unwrap_or_default();
            let survivors = self.field.advance_candidates_by_interval(expected);
            self.field.search_exhausted = survivors == 0;
            if survivors == 0 {
                self.field
                    .set_candidate_not_found_status("候補が見つかりません");
                self.play_candidate_not_found();
            } else if survivors == 1 {
                self.field.mark_timeline_ready();
                let unique_status =
                    "現在位置を一意に特定しました。追加の瞬き入力は停止しています。";
                if self.field.can_start_timer()
                    && self.field.start_blink_timer(self.state.fps_value()).is_ok()
                {
                    self.field.set_info_status(unique_status);
                } else {
                    self.field
                        .set_actionable_status(format!("{unique_status}（再観測してください）"));
                }
                self.play_candidate_confirmed();
            } else {
                self.field.set_info_status(format!(
                    "{survivors}件の候補があります。位置が一意になるまで観測を続けてください。"
                ));
            }
        } else {
            if had_candidates {
                self.field.invalidate_search_results("");
            }
            if self.field.intervals.len() >= 2 && !search_running {
                self.start_field_search(ctx);
            } else if search_running {
                self.jobs.field_search_needs_restart = true;
            }
        }
    }

    fn start_observation_and_generation(&mut self, ctx: &egui::Context) {
        if self.jobs.generation.is_some() {
            self.state.status = "タイムラインを計算中です。観測を続けてください。".into();
            return;
        }
        let config = match self.state.search_config() {
            Ok(config) => config,
            Err(message) => {
                self.state.status = message;
                return;
            }
        };
        // 起動時の準備に失敗していた環境では、観測開始時に再試行する。
        if self.beep_enabled && self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        self.state.start_observation_pending();
        let repaint = ctx.clone();
        self.jobs.generation = Some(std::thread::spawn(move || {
            let cache = crate::domain::rng::generate_observation_cache(
                config.seed,
                config.range_start..=config.range_end,
                config.target,
                config.fps,
            );
            repaint.request_repaint();
            cache
        }));
    }

    fn poll_generation(&mut self) {
        let finished = self
            .jobs
            .generation
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let handle = self.jobs.generation.take().expect("generation handle");
        match handle.join() {
            Ok(Ok(cache)) => {
                if !self.state.is_observing() {
                    // クリックでキャンセルした検索が次のフレームで完了することがある。
                    // その結果を待機画面へ再適用しない。
                    return;
                }
                self.state.apply_observation_cache(cache);
                self.state.rematch_observations();
                if self.state.search_exhausted {
                    self.play_candidate_not_found();
                }
            }
            Ok(Err(message)) => {
                if self.state.is_observing() {
                    self.state.status = message;
                }
            }
            Err(_) => {
                if self.state.is_observing() {
                    self.state.status =
                        "タイムライン計算に失敗しました。検索範囲を確認してください。".into();
                }
            }
        }
    }

    fn poll_field_generation(&mut self, ctx: &egui::Context) {
        let finished = self
            .jobs
            .field_generation
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let handle = self
            .jobs
            .field_generation
            .take()
            .expect("FieldTimeline handle");
        let cancelled = self
            .jobs
            .field_generation_cancel
            .take()
            .is_some_and(|token| token.load(Ordering::Acquire));
        if cancelled {
            let _ = handle.join();
            return;
        }
        match handle.join() {
            Ok(Ok((outcome, pool))) => {
                if self.jobs.field_search_needs_restart {
                    self.jobs.field_search_needs_restart = false;
                    self.jobs.field_pool = Some(pool);
                    if self.field.is_observing()
                        && self.field.intervals.len()
                            >= crate::application::state::MIN_BLINK_INTERVALS
                    {
                        self.field.set_info_status("瞬き間隔を照合中…");
                        self.start_field_search(ctx);
                    }
                    return;
                }
                if !self.field.accepts_search_result() {
                    // キャンセルはクリック操作だけで行う。実行中の検索結果を
                    // キャンセル済みセッションへ再適用しない。
                    // Targetだけの再評価は、既存の瞬きタイマーを動かしたまま別経路で受け付ける。
                    return;
                }
                self.jobs.field_pool = Some(pool);
                // Targetだけの再評価で観測候補を置き換えない。Timeline、開始位置、remain状態、
                // 観測アンカーはTargetから独立しており、到達可否とTargetまでの時間だけを更新する。
                if self.field.target_research_pending && self.field.results.len() == 1 {
                    if let (Some(current), Some(refined)) = (
                        self.field.results.get_mut(self.field.selected_result),
                        outcome.results.first(),
                    ) {
                        current.target_wait_frame = refined.target_wait_frame;
                        current.target_exact = refined.target_exact;
                        current.target_frame_before = refined.target_frame_before;
                        current.target_frame_after = refined.target_frame_after;
                        current.target_blink_count = refined.target_blink_count;
                    }
                    self.field.target_research_pending = false;
                    self.field.search_exhausted = false;
                    self.field.clear_status();
                    // 観測済みTimelineは変わらないため、瞬きタイマーと候補確定音を再開しない。
                    return;
                }
                self.field.target_research_pending = false;
                self.field.results = outcome.results;
                self.field.search_exhausted = self.field.results.is_empty();
                self.jobs.field_timeline_extension_blocked = false;
                self.field.idx_fallback_indices.clear();
                self.field.table_last_followed_row = None;
                self.field.refresh_nearest_timeline_frame();
                if self.field.results.is_empty() {
                    self.play_candidate_not_found();
                }
                let result_status = if self.field.results.is_empty() {
                    "候補が見つかりません".into()
                } else if self.field.results.len() == 1 {
                    self.field.mark_timeline_ready();
                    "現在位置を一意に特定しました。追加の瞬き入力は停止しています。".into()
                } else {
                    let count = self.field.results.len();
                    format!("{count}件の候補があります。位置が一意になるまで観測を続けてください。")
                };
                if self.field.results.is_empty() {
                    self.field.set_candidate_not_found_status(result_status);
                } else {
                    self.field.set_info_status(result_status);
                }
                if self.field.results.len() == 1 {
                    let unique_status = self.field.status.clone();
                    // Enter/ずれ検証後は、遅れて届いた検索結果でも
                    // タイマーや候補確定音を再開しない。リセットでのみ
                    // 新しい観測セッションへ戻る。
                    if self.field.is_correction_mode() {
                        return;
                    }
                    // Timelineが一意になったら瞬き合わせのカウントダウンを開始する。
                    // 候補確定音はタイマー生成とは独立して鳴らす。検索中に予測瞬き時刻を
                    // 過ぎていても、候補確定を音で知らせる必要がある。
                    let timer_started = self.field.can_start_timer()
                        && self.field.start_blink_timer(self.state.fps_value()).is_ok();
                    self.play_candidate_confirmed();
                    if timer_started {
                        self.field.set_info_status(unique_status);
                    } else {
                        self.field.set_actionable_status(format!(
                            "{unique_status}（再観測してください）"
                        ));
                    }
                }
            }
            Ok(Err(message)) => {
                self.field.set_actionable_status(message);
            }
            Err(_) => {
                self.field
                    .set_actionable_status("タイムライン仕様の計算に失敗しました。");
            }
        }
    }

    fn poll_field_pool_generation(&mut self, _ctx: &egui::Context) {
        let finished = self
            .jobs
            .field_pool_generation
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let handle = self
            .jobs
            .field_pool_generation
            .take()
            .expect("FieldTimeline pool handle");
        let cancelled = self
            .jobs
            .field_pool_generation_cancel
            .take()
            .is_some_and(|token| token.load(Ordering::Acquire));
        if cancelled {
            let _ = handle.join();
            return;
        }
        match handle.join() {
            Ok(Ok(pool)) => {
                if self.field.is_observing() {
                    self.jobs.field_pool = Some(pool);
                }
            }
            Ok(Err(message)) => {
                self.field.set_actionable_status(message);
            }
            Err(_) => {
                self.field
                    .set_actionable_status("SFMTプールの計算に失敗しました。");
            }
        }
    }

    /// SFMTプール完成後のTimeline索引を別ワーカーで先行生成する。
    ///
    /// この処理は観測入力や検索ワーカーと同じJoinHandleへ入れない。観測中に
    /// 完了すれば検索で再利用でき、間に合わない場合でも検索自体は詳細照合へ
    /// 進められる。リセット／検索開始時にはキャンセルして古い索引を破棄する。
    #[allow(dead_code)]
    fn start_field_timeline_generation(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeline_generation.is_some() {
            return;
        }
        let Some(pool) = self.jobs.field_pool.as_ref() else {
            return;
        };
        let Ok(configs) = self.field.configs() else {
            return;
        };
        let cache = pool.timeline_cache();
        let repaint = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.field_timeline_generation_cancel = Some(cancel.clone());
        self.jobs.field_timeline_generation = Some(std::thread::spawn(move || {
            let result = crate::application::field::prepare_timeline_warmup_cancelable(
                &cache,
                &configs,
                Some(cancel.as_ref()),
            );
            repaint.request_repaint();
            result
        }));
    }

    fn poll_field_timeline_generation(&mut self) {
        let finished = self
            .jobs
            .field_timeline_generation
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let handle = self
            .jobs
            .field_timeline_generation
            .take()
            .expect("FieldTimeline Timeline handle");
        let cancelled = self
            .jobs
            .field_timeline_generation_cancel
            .take()
            .is_some_and(|token| token.load(Ordering::Acquire));
        if cancelled {
            let _ = handle.join();
            return;
        }
        match handle.join() {
            Ok(Ok(index)) => {
                if self.field.is_observing() {
                    if let Some(pool) = self.jobs.field_pool.as_mut() {
                        pool.install_timeline(index);
                    }
                }
            }
            Ok(Err(message)) if message != "検索をキャンセルしました。" => {
                // Timeline索引は先行最適化なので、失敗しても詳細検索は継続する。
                self.field.set_actionable_status(message);
            }
            Ok(Err(_)) | Err(_) => {}
        }
    }

    /// 一意候補のTimeline延長はModelStatusとSFMTを進めるため、UIスレッドで
    /// 実行しない。1チャンクずつワーカーへ渡し、完了した状態だけを同じ候補へ
    /// 反映する。
    fn start_field_timeline_extension(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeline_extension.is_some()
            || self.jobs.field_timeline_extension_blocked
            || !self.field.timeline_extension_needed(self.state.fps_value())
        {
            return;
        }
        let Some(result) = self.field.results.get(self.field.selected_result) else {
            return;
        };
        let key = (
            self.field.selected_result,
            result.start_consumption,
            result.last_wait_frame,
        );
        let snapshot = self.field.clone();
        let fps = self.state.fps_value();
        let repaint = ctx.clone();
        self.jobs.field_timeline_extension_key = Some(key);
        self.jobs.field_timeline_extension = Some(std::thread::spawn(move || {
            let mut extended = snapshot;
            let appended = extended.extend_timeline_if_needed(fps);
            repaint.request_repaint();
            (extended, appended)
        }));
    }

    fn poll_field_timeline_extension(&mut self) {
        let finished = self
            .jobs
            .field_timeline_extension
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let Some(handle) = self.jobs.field_timeline_extension.take() else {
            return;
        };
        let key = self.jobs.field_timeline_extension_key.take();
        let Ok((extended, appended)) = handle.join() else {
            return;
        };
        let current_key = self
            .field
            .results
            .get(self.field.selected_result)
            .map(|result| {
                (
                    self.field.selected_result,
                    result.start_consumption,
                    result.last_wait_frame,
                )
            });
        if key.is_some() && key == current_key {
            self.field.apply_timeline_extension(extended);
            self.jobs.field_timeline_extension_blocked = !appended;
        }
    }

    fn poll_field_idx_inference(&mut self, _ctx: &egui::Context) {
        let finished = self
            .jobs
            .field_idx_inference
            .as_ref()
            .map(JoinHandle::is_finished)
            .unwrap_or(false);
        if !finished {
            return;
        }
        let handle = self
            .jobs
            .field_idx_inference
            .take()
            .expect("FieldTimeline idx inference handle");
        let cancelled = self
            .jobs
            .field_idx_inference_cancel
            .take()
            .is_some_and(|token| token.load(Ordering::Acquire));
        if cancelled {
            let _ = handle.join();
            return;
        }
        match handle.join() {
            Ok(Ok(inference)) => {
                if self.field.intervals.len() < crate::application::state::MIN_BLINK_INTERVALS {
                    return;
                }
                self.field.inferred_indices = inference.indices.clone();
                self.field.correction_calculated = true;
                self.field.correction_suggested_offset = inference.suggested_offset;
                let unknown = inference.indices.is_empty() || inference.indices.len() > 5;
                self.correction_dialog = Some(CorrectionDialog::ConfirmFieldCorrection {
                    indices: if unknown {
                        Vec::new()
                    } else {
                        inference.indices
                    },
                    unknown,
                    suggested_offset: inference.suggested_offset,
                });
                self.field.clear_status();
            }
            Ok(Err(message)) => self.field.set_actionable_status(message),
            Err(_) => self.field.set_actionable_status("ずれ補正に失敗しました。"),
        }
    }

    fn cancel_field_idx_inference(&mut self) {
        if let Some(token) = self.jobs.field_idx_inference_cancel.as_ref() {
            token.store(true, Ordering::Release);
        }
        self.field.set_info_status("ずれ補正をキャンセル中…");
    }

    fn start_field_pool_generation(&mut self, ctx: &egui::Context) {
        let configs = match self.field.configs() {
            Ok(configs) => configs,
            Err(message) => {
                self.field.set_search_range_status(message);
                return;
            }
        };
        let repaint = ctx.clone();
        self.field.set_info_status("SFMT・Timelineを準備中…");
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.field_pool_generation_cancel = Some(cancel.clone());
        self.jobs.field_pool_generation = Some(std::thread::spawn(move || {
            let result = crate::application::field::prepare_search_pool_with_configs_cancelable(
                &configs,
                Some(cancel.as_ref()),
            );
            repaint.request_repaint();
            result
        }));
    }

    fn start_field_idx_inference(&mut self, ctx: &egui::Context) {
        if self.jobs.field_idx_inference.is_some() || self.field.timer_is_running() {
            return;
        }
        let Ok(configs) = self.field.configs() else {
            return;
        };
        let Ok(actual_frame) = self.field.actual_frame.trim().parse::<i64>() else {
            return;
        };
        let Some(selected) = self.field.results.get(self.field.selected_result).cloned() else {
            return;
        };
        let Some(seed) = configs.first().map(|config| config.seed) else {
            return;
        };
        let tolerance = configs.first().map(|config| config.tolerance).unwrap_or(0);
        let target = configs.first().and_then(|config| config.target_consumption);
        let current_offset = if self.field.use_offset {
            self.field
                .encounter_offset
                .trim()
                .parse::<i64>()
                .unwrap_or_default()
        } else {
            0
        };
        if actual_frame < 0 || self.field.results.len() != 1 {
            return;
        }
        if self.field.correction_calculated {
            let indices = self.field.inferred_indices.clone();
            let unknown = indices.is_empty() || indices.len() > 5;
            self.correction_dialog = Some(CorrectionDialog::ConfirmFieldCorrection {
                indices: if unknown { Vec::new() } else { indices },
                unknown,
                suggested_offset: self.field.correction_suggested_offset,
            });
            return;
        }
        self.field.idx_fallback_indices.clear();
        self.field.set_info_status("ずれ補正を計算中…");
        let repaint = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.field_idx_inference_cancel = Some(cancel.clone());
        self.jobs.field_idx_inference = Some(std::thread::spawn(move || {
            let result = crate::application::field::infer_idx_from_result_cancelable(
                seed,
                &selected,
                tolerance,
                actual_frame,
                Some(cancel.as_ref()),
            );
            let Ok(mut inference) = result else {
                repaint.request_repaint();
                return result;
            };
            inference.suggested_offset = target.and_then(|target| {
                crate::application::field::suggest_field_offset_from_result_cancelable(
                    seed,
                    &selected,
                    target,
                    actual_frame,
                    current_offset,
                    Some(cancel.as_ref()),
                )
                .ok()
                .flatten()
            });
            repaint.request_repaint();
            Ok(inference)
        }));
    }

    fn start_field_search(&mut self, ctx: &egui::Context) {
        let mut configs = match self.field.configs() {
            Ok(configs) => configs,
            Err(message) => {
                self.field.set_search_range_status(message);
                return;
            }
        };
        // 瞬き候補が一意になった後のTarget編集では、その候補のモデル数と
        // 観測idxを再利用する。NPC範囲を再展開すると別設定の先頭結果で
        // 選択済みTimelineを置き換えるためである。
        if self.field.target_research_pending && self.field.results.len() == 1 {
            if let Some(selected) = self.field.results.get(self.field.selected_result) {
                let models = selected.models;
                let observed_model = selected.observed_model;
                configs.retain(|config| config.models == models);
                if configs.is_empty() {
                    if let Some(mut fallback) = self
                        .field
                        .configs()
                        .ok()
                        .and_then(|mut values| values.pop())
                    {
                        fallback.models = models;
                        fallback.target_model = observed_model;
                        configs.push(fallback);
                    }
                }
                for config in &mut configs {
                    config.target_model = observed_model;
                }
            }
        }
        // 先行Timelineは観測入力に間に合わなかった場合、検索とCPUを奪い合わない
        // ように止める。完了済みならpoll側でプールへ取り込まれている。
        if let Some(token) = self.jobs.field_timeline_generation_cancel.take() {
            token.store(true, Ordering::Release);
        }
        self.jobs.field_timeline_generation.take();
        self.jobs.field_timeline_extension.take();
        self.jobs.field_timeline_extension_key.take();
        self.jobs.field_timeline_extension_blocked = false;
        let target_mode = self.field.target_mode;
        self.field.set_info_status("瞬き間隔を照合中…");
        let cached_pool = self
            .jobs
            .field_pool
            .take()
            .filter(|pool| configs.iter().all(|config| pool.covers(config)));
        let pending_pool = self.jobs.field_pool_generation.take();
        let pending_pool_cancel = self.jobs.field_pool_generation_cancel.take();
        let cancel = pending_pool_cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let repaint = ctx.clone();
        self.jobs.field_generation_cancel = Some(cancel.clone());
        self.jobs.field_generation = Some(std::thread::spawn(move || {
            if cancel.load(Ordering::Acquire) {
                return Err("検索をキャンセルしました。".into());
            }
            let pool = if let Some(pool) = cached_pool {
                Ok(pool)
            } else if let Some(handle) = pending_pool {
                let pool = handle
                    .join()
                    .map_err(|_| "SFMTプールの計算に失敗しました。".to_string())??;
                if configs.iter().all(|config| pool.covers(config)) {
                    Ok(pool)
                } else {
                    crate::application::field::prepare_search_pool_with_configs_cancelable(
                        &configs,
                        Some(cancel.as_ref()),
                    )
                }
            } else {
                crate::application::field::prepare_search_pool_with_configs_cancelable(
                    &configs,
                    Some(cancel.as_ref()),
                )
            }?;
            let result = crate::application::field::search_with_npc_range_with_pool_cancelable(
                &configs,
                &pool,
                target_mode,
                Some(cancel.as_ref()),
            )?;
            repaint.request_repaint();
            Ok((result, pool))
        }));
    }
}

// ---------------------------------------------------------------------------
// 孵化タブの観測操作・補正・状態遷移
// ---------------------------------------------------------------------------

fn observation_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    color_scheme: ColorScheme,
    texts: &Texts,
) -> (bool, bool, bool) {
    ui.heading(texts.observation());

    let mut start_requested = false;
    let mut finish_requested = false;
    let mut observe_requested = false;
    ui.add_space(3.0);
    if state.is_observing() {
        if wide_button(ui, true, texts.cancel_observation()) {
            state.cancel_observation();
        }
    } else if state.is_ready()
        || state.is_finished()
        || state.is_timer_phase()
        || state.session_phase() == crate::application::state::SessionPhase::Correction
    {
        if wide_button(ui, true, texts.reset_observation()) {
            state.cancel_observation();
        }
    } else if state.is_idle() && wide_button(ui, true, texts.start_observation()) {
        start_requested = true;
    }

    // 観測条件はセッション開始前に確定し、観測中・候補確定後は固定する。次の観測を
    // やり直す場合はリセットから新しいセッションを開始する。TargetはEnter前まで
    // 別途編集でき、本番タイマー開始後は条件と計算結果が食い違わないよう固定する。
    let range_editable = state.can_edit_observation_settings();
    ui.add_enabled_ui(range_editable, |ui| {
        ui.horizontal(|ui| {
            ui.label(texts.range());
            numeric_text_edit(ui, &mut state.range_start, 72.0, false, false);
            ui.label(texts.range_separator());
            numeric_text_edit(ui, &mut state.range_end, 72.0, false, false);
            ui.label(texts.target_minus());
            numeric_text_edit(ui, &mut state.range_before_target, 62.0, false, false);
            if ui.button(texts.set()).clicked() {
                state.set_range_from_target();
            }
        });
        ui.horizontal(|ui| {
            ui.label(texts.auto_tolerance());
            numeric_text_edit(ui, &mut state.tolerance, 72.0, false, false);
        });
    });

    let intervals_label = if state.intervals.is_empty() {
        format!("{}: —", texts.observation_intervals_frames())
    } else {
        format!(
            "{}: {}",
            texts.observation_intervals_frames(),
            state
                .intervals
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    // 最初の画面からこの行を表示し、最初の間隔を記録してもパネルの配置を変えない。
    let intervals_width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(intervals_width, 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add(
                egui::Label::new(intervals_label.clone())
                    .truncate()
                    .halign(egui::Align::LEFT),
            )
            .on_hover_text(intervals_label);
        },
    );
    if state.search_exhausted {
        ui.colored_label(color_scheme.colors_for_ui(ui).error, texts.error_cancel());
    }

    // 本番／ずれ検証への入口は常に同じ最下段に表示し、Ready以外では
    // グレーアウトする。キーボードEnterと同じ遷移だけをここから行う。
    let transition_is_blink_input = state.is_idle() || state.is_observing();
    let transition_label = if transition_is_blink_input {
        texts.record_blink()
    } else if state.fastest_close {
        texts.correction_start()
    } else {
        texts.production_start()
    };
    let panel_rect = ui.max_rect();
    let transition_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + 4.0, panel_rect.bottom() - 36.0),
        egui::vec2((panel_rect.width() - 8.0).max(0.0), 30.0),
    );
    let mut transition_clicked = false;
    let transition_enabled = if state.is_observing() {
        true
    } else if state.fastest_close {
        state.can_finish_fastest_close_for_correction()
    } else {
        state.is_ready()
    };
    ui.add_enabled_ui(transition_enabled, |ui| {
        transition_clicked = ui
            .put(
                transition_rect,
                egui::Button::new(transition_label).min_size(egui::vec2(
                    transition_rect.width(),
                    transition_rect.height(),
                )),
            )
            .clicked();
    });
    if transition_clicked {
        if state.is_observing() {
            observe_requested = true;
        } else if state.fastest_close {
            finish_requested = true;
        } else {
            state.begin_encounter();
        }
    }
    (start_requested, finish_requested, observe_requested)
}

fn correction_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    input: &mut String,
    wheel_offset_enabled: bool,
    texts: &Texts,
) {
    let panel_rect = ui.max_rect();
    let label_rect = egui::Rect::from_min_size(
        panel_rect.min + egui::vec2(2.0, 4.0),
        egui::vec2(72.0, 24.0),
    );
    ui.put(label_rect, egui::Label::new(texts.correction()));

    let spinner_rect = egui::Rect::from_center_size(
        egui::pos2(panel_rect.center().x, panel_rect.center().y),
        egui::vec2(156.0, 32.0),
    );
    // 位相補正は固定中のSeed/NPC条件から独立し、瞬き観測とタイマー合わせの間も利用できる。
    let enabled = !state.is_timer_phase();
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(spinner_rect), |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            horizontal_frame_spinner(ui, input, 52.0, 1, wheel_offset_enabled);
        });
    });
    if let Ok(value) = input.trim().parse::<i64>() {
        state.set_adjust(value);
    } else if !enabled {
        *input = state.adjust.to_string();
    }
}

fn input_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    wheel_offset_enabled: bool,
    correction_calculating: bool,
    texts: &Texts,
) -> (bool, bool, bool, bool) {
    ui.heading(texts.conditions());
    let before = (
        state.seed.clone(),
        state.target.clone(),
        state.npc.clone(),
        state.encounter_offset.clone(),
        state.use_offset,
        state.consider_rotom_talk,
        state.rotom_threshold.clone(),
        state.consider_pre_rotom_consumption,
        state.pre_rotom_consumption.clone(),
        state.observed_rotom_talk,
        state.consider_npc_initial_load,
        state.npc_initial_load.clone(),
        state.fastest_close,
    );
    let mut correct_offset = false;
    let mut open_manager = false;
    let mut observed_talk_changed = false;
    let mut correction_source_changed = false;
    let mut target_committed = false;
    // 観測開始後は観測したSFMT位置を別モデル条件へ流用しないためSeed/NPCを固定する。
    // その他の条件は本番タイマー開始まで編集できる。
    let seed_npc_editable = state.is_idle();
    let condition_editable = state.can_edit_general_settings();
    // Enterで本番区間を終えた後は補正専用欄だけを編集できる。リセットまで一般設定を
    // 固定し、入力変更でタイマーを再開したり候補確定音を二重に鳴らしたりしない。
    let correction_inputs_editable = state.can_edit_correction_inputs();
    let runtime_condition_editable = condition_editable || correction_inputs_editable;
    let correction_action_editable = condition_editable || correction_inputs_editable;
    // ロトムのお喋り状態は選択済みTimelineから導出する。瞬き特定中は候補が仮状態なので
    // 操作を無効にし、途中候補で表示状態を変えない。Timelineが一意のReadyになった後だけ
    // 操作可能にする。
    let rotom_prediction_ready = state.candidates.len() == 1 && !correction_calculating;
    // 左列に基本条件、右列中央にオフセット／最速閉じを置く。
    // これらの操作だけをパネル中央から始め、縦幅を増やさずに配置する。
    ui.columns(2, |columns| {
        columns[0].vertical(|ui| {
            ui.add_enabled_ui(seed_npc_editable, |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.initial_seed());
                    ui.add(egui::TextEdit::singleline(&mut state.seed).desired_width(120.0));
                });
            });
            ui.add_enabled_ui(condition_editable, |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.target_frame());
                    let (target_changed, committed) = nonnegative_frame_spinner_with_commit(
                        ui,
                        &mut state.target,
                        120.0,
                        1,
                        wheel_offset_enabled,
                    );
                    if target_changed {
                        state.target_dirty = true;
                    }
                    if committed && state.target_dirty {
                        state.target_dirty = false;
                        target_committed = true;
                    }
                });
            });
            ui.add_enabled_ui(seed_npc_editable, |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.model_count());
                    npc_spinner(ui, &mut state.npc, 120.0, wheel_offset_enabled);
                });
            });
        });
        columns[1].with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_enabled_ui(runtime_condition_editable, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.use_offset, texts.offset());
                    ui.add_enabled_ui(state.use_offset, |ui| {
                        let _ = offset_spinner_with_commit(
                            ui,
                            &mut state.encounter_offset,
                            74.0,
                            2,
                            wheel_offset_enabled,
                            0,
                            crate::application::state::MAX_ENCOUNTER_OFFSET_FRAMES,
                        );
                        ui.label("F");
                        if ui.button("＋").clicked() {
                            open_manager = true;
                        }
                    });
                });
            });
            ui.add_enabled_ui(runtime_condition_editable, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.fastest_close, texts.fastest_close());
                });
            });
        });
    });

    ui.separator();
    ui.add_enabled_ui(runtime_condition_editable, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(3.0);
            ui.checkbox(&mut state.consider_rotom_talk, texts.rotom_probability());
            ui.add_enabled_ui(state.consider_rotom_talk, |ui| {
                ui.label(texts.probability());
                numeric_text_edit(ui, &mut state.rotom_threshold, 48.0, false, false);
                ui.label("%");
            });
        });
        ui.horizontal(|ui| {
            ui.add_space(3.0);
            ui.checkbox(
                &mut state.consider_npc_initial_load,
                texts.npc_initial_load(),
            );
            ui.label(texts.model_count());
            ui.add_enabled_ui(state.consider_npc_initial_load, |ui| {
                npc_spinner(ui, &mut state.npc_initial_load, 52.0, wheel_offset_enabled);
            });
        });
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        if correction_calculating {
            ui.add_sized(COMPACT_ACTION_SIZE, egui::Spinner::new());
        } else if ui
            .add_enabled(
                correction_action_editable,
                egui::Button::new(texts.correction())
                    .min_size(egui::vec2(COMPACT_ACTION_SIZE[0], COMPACT_ACTION_SIZE[1])),
            )
            .clicked()
        {
            correct_offset = true;
        }
        ui.label(texts.actual_reached());
        ui.add_enabled_ui(correction_inputs_editable, |ui| {
            numeric_text_edit(ui, &mut state.actual_target_frame, 100.0, false, false);
        });
        ui.label(texts.talk());
        let predicted = if correction_inputs_editable {
            state.predicted_rotom_talk_for_correction_source()
        } else {
            rotom_prediction_ready
                .then(|| state.predicted_rotom_talk())
                .flatten()
        };
        let mut observed = state.observed_rotom_talk.or(predicted).unwrap_or(false);
        let mut observed_changed = false;
        ui.add_enabled_ui(
            correction_inputs_editable && state.consider_rotom_talk,
            |ui| {
                egui::ComboBox::from_id_salt("observed-rotom-talk")
                    .selected_text(if observed {
                        texts.talk_yes()
                    } else {
                        texts.talk_no()
                    })
                    .show_ui(ui, |ui| {
                        observed_changed |= ui
                            .selectable_value(&mut observed, true, texts.talk_yes())
                            .changed();
                        observed_changed |= ui
                            .selectable_value(&mut observed, false, texts.talk_no())
                            .changed();
                    });
            },
        );
        if observed_changed {
            state.set_observed_rotom_talk(observed);
            observed_talk_changed = true;
        }
    });
    // 開始位置は横幅を消費する補正欄から分離し、候補が複数ある場合も
    // コンボボックスが右端へ食い込まない独立した行で選択する。
    ui.horizontal(|ui| {
        ui.label(texts.starting_frame_label());
        let source_frames = if state.is_blink_phase() {
            Vec::new()
        } else {
            state.correction_start_candidates()
        };
        if source_frames.is_empty() {
            ui.label("—");
        } else {
            let mut source_index = state
                .correction_source_index()
                .min(source_frames.len().saturating_sub(1));
            ui.add_enabled_ui(correction_inputs_editable, |ui| {
                egui::ComboBox::from_id_salt("correction-source-frame")
                    .selected_text(source_frames[source_index].to_string())
                    .show_ui(ui, |ui| {
                        for (index, frame) in source_frames.iter().enumerate() {
                            correction_source_changed |= ui
                                .selectable_value(&mut source_index, index, frame.to_string())
                                .changed();
                        }
                    });
            });
            if correction_source_changed {
                state.set_correction_source_index(source_index);
            }
        }
    });
    // 開始位置または確率の変更時だけ、補正欄のロトム初期選択を再同期する。
    // 手動で有無を選んだ同一フレームでは、ユーザーの選択を優先する。
    let rotom_threshold_changed = before.6 != state.rotom_threshold;
    if correction_inputs_editable
        && (correction_source_changed || rotom_threshold_changed)
        && !observed_talk_changed
        && state.sync_observed_rotom_to_correction_source()
    {
        observed_talk_changed = true;
    }
    let after = (
        state.seed.clone(),
        state.target.clone(),
        state.npc.clone(),
        state.encounter_offset.clone(),
        state.use_offset,
        state.consider_rotom_talk,
        state.rotom_threshold.clone(),
        state.consider_pre_rotom_consumption,
        state.pre_rotom_consumption.clone(),
        state.observed_rotom_talk,
        state.consider_npc_initial_load,
        state.npc_initial_load.clone(),
        state.fastest_close,
    );
    let other_changed = before.0 != after.0
        || before.2 != after.2
        || before.3 != after.3
        || before.4 != after.4
        || before.5 != after.5
        || before.6 != after.6
        || before.7 != after.7
        || before.8 != after.8
        || before.9 != after.9
        || before.10 != after.10
        || before.11 != after.11
        || before.12 != after.12;
    (
        other_changed || target_committed,
        correct_offset,
        open_manager,
        observed_talk_changed,
    )
}
