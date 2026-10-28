use super::UiApp;
use crate::application::defaults;
use crate::application::field::FieldTargetMode;
use crate::infrastructure::persistence::{
    PersistedAppState, PersistedField, PersistedHatch, PersistedNormalTimer, PersistedTimeleap,
    PersistedTimeleapCourier, PersistedTimeleapFrame1Search, PersistedTimeleapOptions,
    PersistedTimeleapSource, PersistedTimeleapTargetSearch,
};
use crate::ui::app_types::AppTab;
use eframe::egui;
use std::time::{Duration, Instant};

impl UiApp {
    // --- Persisted input snapshot conversion ------------------------------

    pub(super) fn persisted_state(&self) -> PersistedAppState {
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
                initial_branch_scan_span: self.field.initial_branch_scan_span.clone(),
                timeleap: PersistedTimeleap {
                    enabled: self.field.timeleap_enabled,
                    source: PersistedTimeleapSource {
                        inputs_initialized: self.field.timeleap_source_inputs_initialized,
                        starting_frame: self.field.timeleap_source_starting_frame.clone(),
                        npc_count: self.field.timeleap_source_npc_count.clone(),
                        frame1: self.field.timeleap_source_frame1.clone(),
                    },
                    frame1_search: PersistedTimeleapFrame1Search {
                        frame1_input: self.field.timeleap_frame1_input.clone(),
                        search_range: self.field.timeleap_search_range.clone(),
                        seconds_min: self.field.timeleap_seconds_min.clone(),
                        seconds_max: self.field.timeleap_seconds_max.clone(),
                        frame1_min: Some(self.field.timeleap_frame1_min.clone()),
                        frame1_max: Some(self.field.timeleap_frame1_max.clone()),
                        filter_mode: self.field.timeleap_filter_mode,
                        grace_min: self.field.timeleap_grace_min.clone(),
                    },
                    target_search: PersistedTimeleapTargetSearch {
                        grace_min: self.field.timeleap_target_grace_min.clone(),
                        generation_start_frame: self.field.timeleap_generation_start_frame.clone(),
                        drift_search_enabled: self
                            .field
                            .confirmed_target_search_form
                            .drift_search_enabled,
                        drift_search_generation_start_frame: self
                            .field
                            .confirmed_target_search_form
                            .drift_search_generation_start_frame
                            .clone(),
                        drift_range_ticks: if self.field.confirmed_target_search_form.initialized {
                            self.field
                                .confirmed_target_search_form
                                .drift_range_ticks
                                .clone()
                        } else {
                            defaults::FIELD_TIMELEAP_DRIFT_RANGE_TICKS.into()
                        },
                        confirmed_grace_min_frames: if self
                            .field
                            .confirmed_target_search_form
                            .initialized
                        {
                            self.field
                                .confirmed_target_search_form
                                .grace_min_frames
                                .clone()
                        } else {
                            "0".into()
                        },
                    },
                    courier: PersistedTimeleapCourier {
                        iv_count: self.field.timeleap_courier_iv_count.clone(),
                        ability_locked: self.field.timeleap_courier_ability_locked,
                        nature_locked: self.field.timeleap_courier_nature_locked,
                        gender_locked: self.field.timeleap_courier_gender_locked,
                        ec: self.field.timeleap_courier_ec.clone(),
                        pid_type: self.field.timeleap_courier_pid_type,
                        pid: self.field.timeleap_courier_pid.clone(),
                        fixed_ivs: self.field.timeleap_courier_fixed_ivs,
                        ability: self.field.timeleap_courier_ability,
                        nature: self.field.timeleap_courier_nature,
                        gender_ratio: self.field.timeleap_courier_gender_ratio,
                        is_egg: self.field.timeleap_courier_is_egg,
                        your_id: self.field.timeleap_courier_your_id,
                        other_info: self.field.timeleap_courier_other_info,
                        tid: self.field.timeleap_courier_tid.clone(),
                        sid: self.field.timeleap_courier_sid.clone(),
                        species_id: self.field.timeleap_courier_species_id,
                    },
                    options: PersistedTimeleapOptions {
                        game_edition: self.field.timeleap_game_edition,
                        mode: self.field.timeleap_mode,
                        is_boy: self.field.timeleap_is_boy,
                        default_is_boy: self.field.timeleap_default_is_boy,
                        no_dex: self.field.timeleap_no_dex,
                        direct_encounter: self.field.timeleap_direct_encounter,
                        consider_delay_enabled: self.field.timeleap_consider_delay_enabled,
                        consider_delay_frames: self.field.timeleap_consider_delay_frames.clone(),
                        search_category: self.field.timeleap_search_category,
                        search_preset: self.field.timeleap_search_preset,
                        search_tab: self.field.timeleap_search_tab,
                    },
                },
                target_model: self.field.target_model.clone(),
                target_mode_known: self.field.target_mode == FieldTargetMode::Known,
                actual_frame: self.field.actual_frame.clone(),
                tolerance: self.field.tolerance_for_persistence().to_owned(),
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

    pub(super) fn apply_persisted_state(&mut self, saved: PersistedAppState) {
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
        self.field.initial_branch_scan_span = field.initial_branch_scan_span;
        let timeleap = field.timeleap;
        self.field.timeleap_enabled = timeleap.enabled;
        self.field.timeleap_source_inputs_initialized = timeleap.source.inputs_initialized;
        self.field.restore_timeleap_source_seed_from_main();
        self.field.timeleap_source_starting_frame = timeleap.source.starting_frame;
        self.field.timeleap_source_npc_count = timeleap.source.npc_count;
        self.field.timeleap_source_frame1 = timeleap.source.frame1;
        self.field.timeleap_frame1_input = timeleap.frame1_search.frame1_input;
        self.field.restore_timeleap_source_frame1_from_main();
        self.field.timeleap_mode = timeleap.options.mode;
        self.field.timeleap_is_boy = timeleap.options.is_boy;
        self.field.timeleap_default_is_boy = timeleap.options.default_is_boy;
        self.field.timeleap_direct_encounter = timeleap.options.direct_encounter;
        self.field.timeleap_search_range = timeleap.frame1_search.search_range;
        self.field.timeleap_seconds_min = timeleap.frame1_search.seconds_min;
        self.field.timeleap_seconds_max = timeleap.frame1_search.seconds_max;
        self.field.timeleap_frame1_min = timeleap.frame1_search.frame1_min.unwrap_or_else(|| {
            if timeleap.frame1_search.filter_mode == 1 {
                self.field.timeleap_seconds_min.clone()
            } else {
                "0".into()
            }
        });
        self.field.timeleap_frame1_max = timeleap.frame1_search.frame1_max.unwrap_or_else(|| {
            if timeleap.frame1_search.filter_mode == 1 {
                self.field.timeleap_seconds_max.clone()
            } else {
                crate::domain::rng::MAX_SFMT_FRAME.to_string()
            }
        });
        self.field.timeleap_filter_mode = timeleap.frame1_search.filter_mode.min(1);
        self.field.timeleap_grace_min = timeleap.frame1_search.grace_min;
        self.field.timeleap_target_grace_min = timeleap.target_search.grace_min;
        self.field.timeleap_generation_start_frame = timeleap.target_search.generation_start_frame;
        self.field.confirmed_target_search_form.initialized = true;
        self.field.confirmed_target_search_form.drift_search_enabled =
            timeleap.target_search.drift_search_enabled;
        self.field
            .confirmed_target_search_form
            .drift_search_generation_start_frame =
            timeleap.target_search.drift_search_generation_start_frame;
        self.field.confirmed_target_search_form.drift_range_ticks =
            timeleap.target_search.drift_range_ticks;
        self.field.confirmed_target_search_form.grace_min_frames =
            timeleap.target_search.confirmed_grace_min_frames;
        self.field.timeleap_no_dex = timeleap.options.no_dex;
        self.field.timeleap_consider_delay_enabled = timeleap.options.consider_delay_enabled;
        self.field.timeleap_consider_delay_frames = timeleap.options.consider_delay_frames;
        self.field.timeleap_game_edition = timeleap.options.game_edition;
        (
            self.field.timeleap_search_category,
            self.field.timeleap_search_preset,
        ) = crate::domain::fixed_rng_catalog::normalize_selection(
            self.field.timeleap_game_edition,
            timeleap.options.search_category,
            timeleap.options.search_preset,
        );
        self.field.timeleap_search_tab = timeleap.options.search_tab.min(3);
        self.field.timeleap_courier_iv_count = timeleap.courier.iv_count;
        self.field.timeleap_courier_ability_locked = timeleap.courier.ability_locked;
        self.field.timeleap_courier_nature_locked = timeleap.courier.nature_locked;
        self.field.timeleap_courier_gender_locked = timeleap.courier.gender_locked;
        self.field.timeleap_courier_ec = timeleap.courier.ec;
        self.field.timeleap_courier_pid_type = timeleap.courier.pid_type;
        self.field.timeleap_courier_pid = timeleap.courier.pid;
        self.field.timeleap_courier_fixed_ivs = timeleap.courier.fixed_ivs;
        self.field.timeleap_courier_ability = timeleap.courier.ability;
        self.field.timeleap_courier_nature = timeleap.courier.nature;
        self.field.timeleap_courier_gender_ratio = timeleap.courier.gender_ratio;
        self.field.timeleap_courier_is_egg = timeleap.courier.is_egg;
        self.field.timeleap_courier_your_id = timeleap.courier.your_id;
        self.field.timeleap_courier_other_info = timeleap.courier.other_info;
        self.field.timeleap_courier_tid = timeleap.courier.tid;
        self.field.timeleap_courier_sid = timeleap.courier.sid;
        self.field.timeleap_courier_species_id = timeleap.courier.species_id;
        self.field.refresh_timeleap_courier_delay();
        self.field.target_model = if field.target_model.trim().is_empty() {
            "0".to_string()
        } else {
            field.target_model
        };
        // フィールドはNPC番号を明示する入力モードだけを使用する。
        // 保存データを読み込む際も、この指定番号の経路へ正規化する。
        self.field.target_mode = FieldTargetMode::Known;
        self.field.actual_frame = field.actual_frame;
        self.field.tolerance = field.tolerance;
        self.field.restore_target_input(field.target_consumption);
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

    pub(super) fn persist_if_changed(&mut self, ctx: &egui::Context) {
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
}
