use crate::application::defaults;
use crate::application::field::{FieldState, FieldTimerKind};
use crate::application::state::AppState;
use crate::errors::AppError;
#[cfg(test)]
use crate::errors::StateIssue;
use crate::infrastructure::audio::AudioOutput;
use crate::infrastructure::persistence::PersistedAppState;
use crate::ui::app_types::*;
use crate::ui::beep_scheduler::BeepScheduler;
use crate::ui::field_view::{field_ui, timeleap_window_keepalive_ui, FieldUiWorkerState};
use crate::ui::hatch_view::hatch_guidance_ui;
use crate::ui::i18n::{Language, Texts};
use crate::ui::layout::{
    split_panel_rects, ACTION_ROW_HEIGHT, LEFT_PANEL_WIDTH, PANEL_GAP,
    PANEL_PADDING as PANEL_INNER_PADDING, TIMER_PANEL_HEIGHT as TIMER_DISPLAY_HEIGHT,
    TOP_PANEL_HEIGHT,
};
use crate::ui::search_jobs::SearchJobs;
use crate::ui::settings_view::settings_ui;
use crate::ui::timer_view::normal_timer_ui;
use crate::ui::widgets::{
    draw_timer_flash, fixed_group_panel, horizontal_frame_spinner, install_app_style,
    install_japanese_font, nonnegative_frame_spinner_with_commit, npc_spinner, numeric_text_edit,
    offset_spinner_with_commit, wide_button, ColorScheme, FlashMode, COMPACT_ACTION_SIZE,
};
use eframe::egui;
use std::time::{Duration, Instant};

mod correction;
mod observation;
mod offset_manager;
mod persistence;
mod timeleap_workers;
mod timeline_workers;
mod timer;
mod worker;

const TIMELINE_HEADER_HEIGHT: f32 = 24.0;

fn keep_timeleap_window_viewport(tab: AppTab, is_open: bool) -> bool {
    tab != AppTab::Field && is_open
}

fn should_refresh_single_field_timeline(result_count: usize, correction_mode: bool) -> bool {
    result_count == 1 && !correction_mode
}

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
    cache_access_alert: Option<String>,
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
    fn present_app_error(&mut self, error: AppError) {
        if error.is_cache_io() {
            self.fail_cache_access(error.to_string());
        }
    }

    fn present_app_error_ref(&mut self, error: &AppError) {
        if error.is_cache_io() {
            self.fail_cache_access(error.to_string());
        }
    }

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
            cache_access_alert: None,
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
        if let Some(error) = self.state.take_deferred_cache_io_error() {
            self.present_app_error(error);
        }
        if self.cache_access_alert.is_some() {
            self.show_cache_access_alert(ctx, &texts);
            return;
        }
        let hatch_was_unique = self.state.candidates.len() == 1;
        let field_was_observing = self.field.is_observing();
        self.poll_generation();
        self.poll_field_pool_generation(ctx);
        self.poll_field_timeline_generation();
        self.poll_field_timeline_extension();
        self.start_field_timeline_extension(ctx);
        self.poll_field_generation(ctx);
        self.poll_field_target_generation(ctx);
        self.poll_field_timeleap_candidates(ctx);
        self.poll_field_timeleap_frame1(ctx);
        self.poll_prepared_timeleap_target(ctx);
        self.ensure_field_timeleap_frame1_preview(ctx);
        self.poll_field_timeleap_frame1_preview(ctx);
        self.poll_field_timeleap_target_check(ctx);
        self.poll_field_timeleap_reachability(ctx);
        self.poll_field_timeleap_shift_resolution(ctx);
        self.poll_field_confirmed_target_search(ctx);
        self.poll_field_idx_inference(ctx);
        self.poll_guidance_generation();
        self.poll_normal_target_wait_fallback();
        self.poll_candidate_reachability();
        self.poll_production_preparation();
        if self.correction_calculating {
            let correction_finished = !self.jobs.correction_generation.is_running()
                || self.jobs.correction_generation.is_finished();
            if correction_finished {
                let generation_key = self.jobs.correction_generation_key.take();
                let mut result_is_current =
                    self.state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
                if let Some((_, result)) = self.jobs.correction_generation.take_finished() {
                    result_is_current &= generation_key.is_some_and(|(seed, start, end, fps)| {
                        self.state.search_config().ok().is_some_and(|config| {
                            config.seed == seed
                                && config.range_start == start
                                && config.range_end == end
                                && config.fps.to_bits() == fps
                        })
                    });
                    match result {
                        Ok(Ok(cache)) if result_is_current => {
                            self.state.apply_observation_cache(cache)
                        }
                        Ok(Err(message)) => {
                            if result_is_current {
                                self.present_app_error(message);
                            }
                        }
                        Err(()) => {}
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
        if self.cache_access_alert.is_some() {
            self.show_cache_access_alert(ctx, &Texts::new(self.language));
            return;
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
        let now = Instant::now();
        if self.tab == AppTab::Field && self.field.tick_timeleap_frame1(now) {
            self.handle_timeleap_frame1_transition(ctx, now);
        }
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
                self.clear_unused_sfmt_cache_files();
            } else if self.field.session_phase() == crate::application::state::SessionPhase::Input
                && self.field.results.is_empty()
                && !self.jobs.field_generation.is_running()
                && !self.jobs.field_pool_generation.is_running()
                && !self.jobs.field_idx_inference.is_running()
                && self.field.configs().is_ok()
            {
                self.field.start_observation();
            }
        } else if global_shortcuts_enabled
            && shift_pressed
            && self.tab == AppTab::Field
            && self.field.timeleap_accepts_shift()
        {
            if let Ok(Some(request)) = self
                .field
                .prepare_timeleap_shift_resolution(Instant::now(), self.state.fps_value())
            {
                let generation = request.generation;
                let repaint = ctx.clone();
                self.jobs
                    .field_timeleap_shift_resolution
                    .spawn_cancellable(move |cancel| {
                        let survivors = request.resolve(Some(cancel.as_ref()));
                        repaint.request_repaint();
                        (generation, survivors)
                    });
            }
        } else if global_shortcuts_enabled
            && shift_pressed
            && self.tab == AppTab::Field
            && self.field.is_observation_input_enabled()
            && !self.jobs.field_idx_inference.is_running()
        {
            self.record_field_blink(ctx);
        } else if global_shortcuts_enabled && self.tab == AppTab::Hatch && space {
            if self.state.is_idle() {
                self.start_observation_and_generation(ctx);
            } else if self.state.is_observing() {
                self.state.cancel_observation();
                self.cancel_observation_generation();
                self.clear_unused_sfmt_cache_files();
            }
        } else if global_shortcuts_enabled && self.tab == AppTab::Hatch && shift_pressed {
            self.record_hatch_blink();
        } else if global_shortcuts_enabled && self.tab == AppTab::NormalTimer && space {
            self.toggle_normal_timer();
        }
        if self.tab == AppTab::Field
            && field_was_observing
            && self.field.is_observing()
            && !self.jobs.field_generation.is_running()
            && !self.jobs.field_pool_generation.is_running()
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
            && !(self.field.timeleap_enabled && self.field.timeleap_succeeded())
            && !self.field.timeleap_frame1_preparing
            && !self.jobs.field_generation.is_running()
            && !self.jobs.field_pool_generation.is_running()
            && !self.jobs.field_idx_inference.is_running()
        {
            if self.field.timeleap_enabled {
                if self.field.timeleap_frame1_waiting() {
                    self.field.cancel_timeleap_ready();
                    self.jobs.cancel_prepared_timeleap_target();
                    self.beeps.field.clear();
                    self.beeps.field_start = None;
                } else if self.field.timeleap_failed() {
                    self.field.timeleap_retry(self.state.fps_value());
                } else if self.field.timeleap_assumed() {
                    if let Err(error) = self.field.begin_timeleap_recheck(
                        self.field.target_is_passed(self.state.fps_value()),
                        self.state.fps_value(),
                    ) {
                        if error.is_cache_io() {
                            self.present_app_error(error);
                        } else {
                            self.field.notice = Some(texts.app_error(&error));
                        }
                    }
                } else if self.field.timeleap_lifecycle_policy().phase
                    == crate::application::field::TimeleapLifecyclePhase::Frame1Setup
                {
                    self.start_field_timeleap_frame1_preparation(ctx);
                }
            } else {
                self.field.enter_correction_mode(self.state.fps_value());
            }
            if !self.field.timeleap_accepts_shift() {
                self.beeps.field.clear();
                self.beeps.field_start = None;
            }
        }
        // タイムライン仕様のタイマーモードは、テキスト入力欄にフォーカスがないときだけ
        // 上下キーで切り替える。入力欄のカーソル操作とは競合させない。
        if self.tab == AppTab::Field && global_shortcuts_enabled && (up || down) {
            if let Err(message) = self.field.toggle_timer_kind(self.state.fps_value()) {
                if self.field.next_blink_out_of_range {
                    self.field
                        .set_search_range_notice(texts.app_error(&message));
                }
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

        if self.tab == AppTab::Hatch && !self.jobs.guidance_generation.is_running() {
            self.start_guidance_generation(ctx);
        }
        self.maybe_start_candidate_reachability(ctx);
        self.maybe_start_production_preparation(ctx);
        if !self.beeps.article.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        // タイマーは表示中タブに依存せず進める。別タブを見ている間も
        // 瞬き／Target到達・beep・区間遷移を失わないようにする。
        // 停止・モード切替時に不要な予約を残さない。
        self.retry_field_timer_if_needed(ctx);
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
                    if self
                        .field
                        .finish_blink_timer_segment(self.state.fps_value())
                        .is_err()
                    {
                        // ちょうどTimelineの末尾で延長処理が走っていると、
                        // 次の行が一時的にまだ見えないことがある。ここで
                        // タイマーを終了扱いにせず、延長後に再試行する。
                        self.field_timer_retry = true;
                    }
                }
                FieldTimerKind::Target => {
                    // タイマーとTimelineは0秒でも保持する。瞬きカウントダウンへの切替や
                    // 補正モードへの移行は明示操作でのみ行い、その操作だけが停止条件になる。
                    self.field.finish_target_timer_if_elapsed(Instant::now());
                }
            }
        }
        let due = BeepScheduler::drain_due(&mut self.beeps.field, Instant::now());
        for _ in 0..due {
            self.play_beep();
        }
        if self.field.timer_is_running()
            || !self.beeps.field.is_empty()
            || should_refresh_single_field_timeline(
                self.field.results.len(),
                self.field.is_correction_mode(),
            )
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
            if self.state.finish_production_if_elapsed(Instant::now()) {
                let due = BeepScheduler::drain_due(&mut self.beeps.article, Instant::now());
                for _ in 0..due {
                    self.play_beep();
                }
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

        let (hatch_guidance_snapshot, hatch_guidance_searching) = if self.tab == AppTab::Hatch {
            let (guidance, searching) = match self.state.hatch_guidance_snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.present_app_error(error);
                    (None, false)
                }
            };
            self.maybe_start_normal_target_wait_fallback(ctx, guidance);
            (guidance, searching)
        } else {
            (None, false)
        };

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
                let production_settings_before = (
                    self.state.consider_pre_rotom_consumption,
                    self.state.pre_rotom_consumption.clone(),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(settings_rect), |ui| {
                    let _ = settings_ui(
                        ui,
                        &mut self.state,
                        &mut self.field,
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
                let production_settings_changed = production_settings_before
                    != (
                        self.state.consider_pre_rotom_consumption,
                        self.state.pre_rotom_consumption.clone(),
                    );
                if production_settings_changed
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
                    FieldUiWorkerState {
                        search_running: self.jobs.field_generation.is_running(),
                        pool_generation_running: self.jobs.field_pool_generation.is_running(),
                        idx_inference_running: self.jobs.field_idx_inference.is_running(),
                        timeleap_search_running: self.jobs.field_timeleap_candidates.is_running(),
                        confirmed_target_search_running: self
                            .jobs
                            .field_confirmed_target_search
                            .is_running(),
                        timeleap_frame1_running: self.jobs.field_timeleap_frame1.is_running(),
                        timeleap_frame1_preview_running: self
                            .jobs
                            .field_timeleap_frame1_preview
                            .is_running(),
                    },
                    ctx,
                    self.state.fps_value(),
                    timer_flashing,
                    self.wheel_offset_enabled,
                    self.color_scheme,
                    &texts,
                    self.state.fastest_close,
                );
                if let Some(error) = self.field.take_timeline_stream_error() {
                    if error.is_cache_io() {
                        self.fail_cache_access(error.to_string());
                        return;
                    }
                }
                if field_actions.timeleap_enabled_changed {
                    self.beeps.field.clear();
                    self.beeps.field_start = None;
                }
                if field_actions.cancelled {
                    self.cancel_field_background_tasks();
                    self.clear_unused_sfmt_cache_files();
                }
                if field_actions.timeleap_search_cancel_requested {
                    self.jobs.cancel_prepared_timeleap_target();
                    self.jobs.field_timeleap_candidates.cancel();
                    self.jobs.field_timeleap_target_check.cancel_and_discard();
                    self.field.clear_timeleap_candidates();
                }
                if field_actions.timeleap_candidate_search_cancel_requested {
                    self.jobs.field_timeleap_candidates.cancel();
                }
                if field_actions.confirmed_target_search_cancel_requested {
                    self.jobs.field_confirmed_target_search.cancel();
                }
                if field_actions.timeleap_frame1_preview_requested {
                    self.start_field_timeleap_frame1_preview(ctx);
                }
                if field_actions.timeleap_reachability_search_requested {
                    self.start_field_timeleap_reachability(ctx);
                }
                if let Some(row) = field_actions.confirmed_target_search_row.as_ref() {
                    if self.field.set_confirmed_target_frame_from_search_row(row) {
                        self.field.prepare_target_research(self.state.fps_value());
                    }
                }
                if let Some(frame1) = field_actions.timeleap_frame1_offset_frame {
                    if self
                        .field
                        .apply_timeleap_frame1_offset(frame1, self.state.fastest_close)
                        .is_some()
                    {
                        self.field
                            .refresh_target_timer_for_offset(self.state.fps_value());
                    }
                }
                if let Some(drift_frames) = field_actions.confirmed_target_offset_drift_frames {
                    if self
                        .field
                        .apply_timeleap_offset_drift(drift_frames, self.state.fastest_close)
                        .is_some()
                    {
                        self.field
                            .refresh_target_timer_for_offset(self.state.fps_value());
                    }
                }
                if field_actions.confirmed_target_search_requested {
                    self.start_field_confirmed_target_search(ctx);
                }
                if field_actions.observe_requested {
                    self.record_field_blink(ctx);
                }
                if field_actions.correction_requested {
                    if self.field.timeleap_enabled && self.field.timeleap_failed() {
                        self.field.timeleap_retry(self.state.fps_value());
                    } else if self.field.timeleap_enabled
                        && (self.field.timeleap_succeeded() || self.field.timeleap_assumed())
                    {
                        if let Err(error) = self.field.begin_timeleap_recheck(
                            self.field.target_is_passed(self.state.fps_value()),
                            self.state.fps_value(),
                        ) {
                            if error.is_cache_io() {
                                self.present_app_error(error);
                            } else {
                                self.field.notice = Some(texts.app_error(&error));
                            }
                        }
                    } else if self.field.timeleap_enabled
                        && self.field.timeleap_lifecycle_policy().phase
                            == crate::application::field::TimeleapLifecyclePhase::Frame1Setup
                    {
                        self.start_field_timeleap_frame1_preparation(ctx);
                    } else if !self.field.timeleap_enabled {
                        self.field.enter_correction_mode(self.state.fps_value());
                    }
                    self.beeps.field.clear();
                    self.beeps.field_start = None;
                }
                if field_actions.timeleap_ready_cancel_requested {
                    self.field.cancel_timeleap_ready();
                    self.jobs.cancel_prepared_timeleap_target();
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
                if field_actions.timeleap_search_requested {
                    self.request_field_timeleap_candidate_search(ctx);
                }
                let observation_search_requested =
                    field_actions.search_requested && self.field.intervals.len() >= 2;
                if (target_refinement_requested || observation_search_requested)
                    && !self.field.target_dirty
                {
                    if !self.jobs.field_generation.is_running()
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
                    && !self.jobs.field_generation.is_running()
                {
                    self.jobs.field_pool_generation.cancel_and_discard();
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
            let (left_rect, right_rect) = split_panel_rects(top_rect, LEFT_PANEL_WIDTH, PANEL_GAP);
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
                    let timer_side_label = if self.state.production_preparation_pending {
                        if self.state.fastest_close {
                            texts.next_circle_calculating().to_string()
                        } else {
                            texts.production_wait_calculating().to_string()
                        }
                    } else if self.state.article_timer_stage
                        == Some(crate::application::state_types::ArticleTimerStage::ToTarget)
                    {
                        // オフセット区間中は次の本番待機時間を予告する。
                        // ToTargetへ移った後は中央タイマーが実際の残り時間を示すため、
                        // 予告ラベルを残さない。
                        String::new()
                    } else {
                        hatch_guidance_snapshot
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
                                            guidance.target_unreachable.then(|| {
                                                texts.production_wait_unknown().to_string()
                                            })
                                        })
                                        .unwrap_or_else(|| "—".into())
                                }
                            })
                            .unwrap_or_else(|| {
                                if !self.state.fastest_close
                                    && self.jobs.normal_target_wait.is_running()
                                {
                                    texts.production_wait_calculating().to_string()
                                } else {
                                    "—".into()
                                }
                            })
                    };

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
                        hatch_guidance_snapshot,
                        self.state.target_dirty,
                        hatch_guidance_searching,
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
            let (bottom_left_rect, bottom_right_rect) =
                split_panel_rects(bottom_rect, LEFT_PANEL_WIDTH, PANEL_GAP);
            let mut encounter_requested = false;
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bottom_left_rect), |ui| {
                fixed_group_panel(ui, |ui| {
                    let (
                        start_requested,
                        finish_requested,
                        observe_requested,
                        cancelled,
                        begin_requested,
                    ) = observation_ui(ui, &mut self.state, self.color_scheme, &texts);
                    encounter_requested = begin_requested;
                    if cancelled {
                        self.cancel_observation_generation();
                        self.clear_unused_sfmt_cache_files();
                    }
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
                    let (
                        changed,
                        correct_offset,
                        open_manager,
                        observed_talk_changed,
                        target_committed,
                    ) = input_ui(
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
                    }
                    if target_committed && !self.state.candidates.is_empty() {
                        self.state.mark_production_preparation_pending();
                    }
                    if open_manager {
                        self.open_offset_manager(OffsetScope::Hatch);
                    }
                    if correct_offset {
                        self.request_correction(ctx);
                    }
                });
            });
            if encounter_requested {
                self.state.begin_encounter();
            }
        });

        if self.cache_access_alert.is_some() {
            self.show_cache_access_alert(ctx, &Texts::new(self.language));
            return;
        }

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

        // マウスの移行ボタンは描画中、キーボードEnterは描画後に状態を変える。
        // すべての操作を処理した後で遷移を検出し、開始beepを取りこぼさない。
        if !was_production && self.state.is_production() {
            self.beeps.article = self.schedule_segment_beeps(
                self.state.timer_start_time(),
                self.state.timer_seconds_value(),
            );
            self.beeps.article_start = self.state.timer_start_time();
        }

        self.show_correction_dialog(ctx, &texts);

        self.show_offset_manager(ctx, &texts);

        // The native Timeleap window is declared by its focused keepalive renderer while
        // another main tab is active; do not redraw the complete Field screen offscreen.
        if keep_timeleap_window_viewport(self.tab, self.field.timeleap_search_window_open) {
            egui::Area::new(egui::Id::new("timeleap-viewport-keepalive"))
                .fixed_pos(egui::pos2(-20_000.0, -20_000.0))
                .show(ctx, |ui| {
                    let field_actions = timeleap_window_keepalive_ui(
                        ui,
                        &mut self.field,
                        FieldUiWorkerState {
                            search_running: self.jobs.field_generation.is_running(),
                            pool_generation_running: self.jobs.field_pool_generation.is_running(),
                            idx_inference_running: self.jobs.field_idx_inference.is_running(),
                            timeleap_search_running: self
                                .jobs
                                .field_timeleap_candidates
                                .is_running(),
                            confirmed_target_search_running: self
                                .jobs
                                .field_confirmed_target_search
                                .is_running(),
                            timeleap_frame1_running: self.jobs.field_timeleap_frame1.is_running(),
                            timeleap_frame1_preview_running: self
                                .jobs
                                .field_timeleap_frame1_preview
                                .is_running(),
                        },
                        ctx,
                        self.state.fps_value(),
                        self.wheel_offset_enabled,
                        self.color_scheme,
                        &texts,
                        self.state.fastest_close,
                    );
                    self.dispatch_keepalive_window_actions(ctx, field_actions);
                });
        }

        self.persist_if_changed(ctx);

        if self.jobs.generation.is_running()
            || self.jobs.field_generation.is_running()
            || self.jobs.field_pool_generation.is_running()
            || self.jobs.field_timeline_generation.is_running()
            || self.jobs.field_timeline_extension.is_running()
            || self.jobs.field_idx_inference.is_running()
            || self.jobs.field_timeleap_shift_resolution.is_running()
            || self.jobs.guidance_generation.is_running()
            || self.jobs.normal_target_wait.is_running()
            || self.jobs.candidate_reachability.is_running()
            || self.jobs.production_preparation.is_running()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl UiApp {
    fn dispatch_keepalive_window_actions(
        &mut self,
        ctx: &egui::Context,
        actions: crate::ui::field_view::FieldUiActions,
    ) {
        if actions.timeleap_search_cancel_requested {
            self.jobs.cancel_prepared_timeleap_target();
            self.jobs.field_timeleap_candidates.cancel();
            self.jobs.field_timeleap_target_check.cancel_and_discard();
            self.field.clear_timeleap_candidates();
        }
        if actions.timeleap_candidate_search_cancel_requested {
            self.jobs.field_timeleap_candidates.cancel();
        }
        if actions.confirmed_target_search_cancel_requested {
            self.jobs.field_confirmed_target_search.cancel();
        }
        if actions.timeleap_frame1_preview_requested {
            self.start_field_timeleap_frame1_preview(ctx);
        }
        if actions.timeleap_reachability_search_requested {
            self.start_field_timeleap_reachability(ctx);
        }
        if let Some(row) = actions.confirmed_target_search_row.as_ref() {
            if self.field.set_confirmed_target_frame_from_search_row(row) {
                self.field.prepare_target_research(self.state.fps_value());
            }
        }
        if let Some(frame1) = actions.timeleap_frame1_offset_frame {
            if self
                .field
                .apply_timeleap_frame1_offset(frame1, self.state.fastest_close)
                .is_some()
            {
                self.field
                    .refresh_target_timer_for_offset(self.state.fps_value());
            }
        }
        if let Some(drift_frames) = actions.confirmed_target_offset_drift_frames {
            if self
                .field
                .apply_timeleap_offset_drift(drift_frames, self.state.fastest_close)
                .is_some()
            {
                self.field
                    .refresh_target_timer_for_offset(self.state.fps_value());
            }
        }
        if actions.confirmed_target_search_requested {
            self.start_field_confirmed_target_search(ctx);
        }
        if actions.timeleap_search_requested {
            self.request_field_timeleap_candidate_search(ctx);
        }
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
) -> (bool, bool, bool, bool, bool) {
    ui.heading(texts.observation());

    let mut start_requested = false;
    let mut finish_requested = false;
    let mut observe_requested = false;
    let mut cancelled = false;
    let mut encounter_requested = false;
    ui.add_space(3.0);
    if state.is_observing() {
        if wide_button(ui, true, texts.cancel_observation()) {
            state.cancel_observation();
            cancelled = true;
        }
    } else if state.is_ready()
        || state.is_finished()
        || state.is_timer_phase()
        || state.session_phase() == crate::application::state::SessionPhase::Correction
    {
        if wide_button(ui, true, texts.reset_observation()) {
            state.cancel_observation();
            cancelled = true;
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
            encounter_requested = true;
        }
    }
    (
        start_requested,
        finish_requested,
        observe_requested,
        cancelled,
        encounter_requested,
    )
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
    ui.put(label_rect, egui::Label::new(texts.blink_correction()));

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

#[cfg(test)]
mod tests;

fn input_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    wheel_offset_enabled: bool,
    correction_calculating: bool,
    texts: &Texts,
) -> (bool, bool, bool, bool, bool) {
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
    // 観測開始後はSeed/NPCを固定する。条件の変更はリセット後に行う。
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
        if ui
            .add_enabled(
                correction_action_editable && !correction_calculating,
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
        other_changed,
        correct_offset,
        open_manager,
        observed_talk_changed,
        target_committed,
    )
}
