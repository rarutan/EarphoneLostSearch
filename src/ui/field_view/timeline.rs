use super::*;

pub(super) fn field_text_scale(available_width: f32) -> f32 {
    (available_width / 1000.0).clamp(1.0, 1.6)
}

pub(super) fn format_timeleap_duration(seconds: i64) -> String {
    let seconds = seconds.clamp(0, TIMELEAP_MAX_DISPLAY_DURATION_SECONDS);
    let hours = seconds / 3600;
    let minutes = seconds / 60 % 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub(super) fn field_timeline_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    calculating: bool,
    fps: f64,
    scale: f32,
    color_scheme: ColorScheme,
    texts: &Texts,
) {
    // Enterで補正モードへ移った後も、タイマーだけを停止して
    // 観測済みTimelineは閲覧できるようにする。
    let timeline_unique = !calculating && state.results.len() == 1;
    let _ = field_timeline_header_ui(ui, state, timeline_unique, scale, texts);
    let Some(result) = state.results.get(state.selected_result) else {
        return;
    };
    if !timeline_unique {
        return;
    }
    if result.timeline.is_empty() {
        return;
    }

    let current_row = state
        .current_timeline_row(fps)
        .unwrap_or(0)
        .min(result.timeline.len().saturating_sub(1));
    let lifecycle = state.timeleap_lifecycle_policy();
    let frame1_table = lifecycle.table_mode == crate::application::field::TimeleapTableMode::Frame1;
    let frame1_limit = lifecycle.frame1_boundary_target;
    // TimelineはFrame順に並んでいるため、Frame1を含む最終行だけ先に一度求める。
    // 行ごとのsuffix走査を避け、各行はこの境界との差だけで表示数を出す。
    let frame1_boundary = frame1_limit.and_then(|frame| {
        result
            .frame_boundary_index(frame)
            .or_else(|| state.timeleap_frame1_table_boundary(frame))
    });
    let target_timeline_frame = result
        .target_wait_frame
        .map(|wait| result.last_wait_frame.saturating_add(wait));
    ui.horizontal(|ui| {
        for (width, label) in [
            (TABLE_SFMT_COLUMN_WIDTH, texts.timeline_sfmt_frame()),
            (
                TABLE_SECONDS_COLUMN_WIDTH,
                texts.timeline_next_blink_seconds(),
            ),
            (
                TABLE_TARGET_BLINKS_COLUMN_WIDTH,
                if frame1_table {
                    texts.timeline_frame1_blink_count()
                } else {
                    texts.timeline_target_blink_count()
                },
            ),
        ] {
            ui.add_sized(
                [width * scale, 20.0 * scale],
                egui::Label::new(egui::RichText::new(label).size(13.0 * scale).strong()),
            );
        }
    });

    let row_height = 22.0 * scale;
    // 表の操作に必要なフッターだけを確保する。固定の大きな予約領域は行の下に
    // 不要な余白を残すため、表示に必要な高さへ限定する。
    let table_height = (ui.available_height() - 26.0 * scale).max(80.0 * scale);
    // スクロール内容の上下に余白を設け、先頭・末尾の行もビューポート中央へ置ける
    // ようにする。通常のスクロール値はeguiが内容範囲で丸めるため端行を中央にできない。
    let center_padding = ((table_height - row_height) * 0.5).max(0.0);
    // 追従モードでは毎フレーム補正し、ホイール操作で青い現在行が中央から外れない
    // ようにする。自由モードでは補正せず、任意の行を確認できるようにする。
    if state.table_follow_current {
        state.table_last_followed_row = Some(current_row);
    }
    let scroll = egui::ScrollArea::vertical()
        .id_salt("field-timeline-scroll")
        .max_height(table_height)
        .min_scrolled_height(table_height)
        .auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        ui.add_space(center_padding);
        for (index, row) in result.timeline.iter().enumerate() {
            let (row_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), row_height),
                egui::Sense::hover(),
            );
            if index == current_row {
                ui.painter()
                    .rect_filled(row_rect, 2.0, color_scheme.colors_for_ui(ui).current_row);
                if state.table_follow_current {
                    // 推定値ではなくeguiの実際の表示領域を使う。スクロールバー、枠の高さ、
                    // DPI、行余白が変わっても正しく、アニメーション1フレームの遅れなく追従する。
                    ui.scroll_to_rect_animation(
                        row_rect,
                        Some(egui::Align::Center),
                        egui::style::ScrollAnimation::none(),
                    );
                }
            }
            ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(row_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.add_sized(
                        [TABLE_SFMT_COLUMN_WIDTH * scale, row_height],
                        egui::Label::new(if index == current_row {
                            format!("▶ {}", row.frame)
                        } else {
                            row.frame.to_string()
                        }),
                    );
                    let seconds = crate::domain::timing::game_frames_to_seconds(
                        row.duration_frames.max(0) as f64,
                        fps,
                    );
                    let seconds_label =
                        if state.next_blink_out_of_range && index + 1 == result.timeline.len() {
                            "—".into()
                        } else {
                            format!("{seconds:.2}s")
                        };
                    ui.add_sized(
                        [TABLE_SECONDS_COLUMN_WIDTH * scale, row_height],
                        egui::Label::new(seconds_label),
                    );
                    let target_blinks = if let Some(frame1) = frame1_limit {
                        frame1_boundary.and_then(|boundary| {
                            result
                                .blinks_through_frame_after_row_at_boundary(index, frame1, boundary)
                        })
                    } else if frame1_table {
                        None
                    } else {
                        target_timeline_frame
                            .filter(|target| *target > row.timeline_frame)
                            .and_then(|_| result.target_blinks_after_row(index))
                    }
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "—".into());
                    ui.add_sized(
                        [TABLE_TARGET_BLINKS_COLUMN_WIDTH * scale, row_height],
                        egui::Label::new(target_blinks),
                    );
                },
            );
        }
        ui.add_space(center_padding);
    });
    field_current_position_ui(ui, state, fps, scale);
}

/// 描画するTimeline見出しを一箇所に保ち、8桁値と追従チェックの実矩形を
/// レイアウト回帰テストからも取得できるようにする。
pub(super) fn field_timeline_header_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    timeline_unique: bool,
    scale: f32,
    texts: &Texts,
) -> (egui::Rect, egui::Rect) {
    let mut summary_rect = egui::Rect::NOTHING;
    let mut has_summary = false;
    let mut checkbox_rect = egui::Rect::NOTHING;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0 * scale;
        let mut include_summary = |rect: egui::Rect| {
            summary_rect = if has_summary {
                summary_rect.union(rect)
            } else {
                rect
            };
            has_summary = true;
        };
        let selected = state.results.get(state.selected_result);
        if timeline_unique {
            if let Some(result) = selected.filter(|result| !result.timeline.is_empty()) {
                if let Some(starting_frame) = state.starting_frame_for_display(result) {
                    let label = texts.starting_frame(starting_frame);
                    let label = ui.label(egui::RichText::new(label).size(14.0 * scale).strong());
                    include_summary(label.rect);
                    let copy = ui.button(texts.copy());
                    include_summary(copy.rect);
                    if copy.clicked() {
                        ui.output_mut(|output| {
                            output.copied_text = starting_frame.to_string();
                        });
                    }
                } else {
                    let label = ui.label(
                        egui::RichText::new("StartingFrame : —")
                            .size(14.0 * scale)
                            .strong(),
                    );
                    include_summary(label.rect);
                }
            } else {
                let label = ui.label(
                    egui::RichText::new("StartingFrame : —")
                        .size(14.0 * scale)
                        .strong(),
                );
                include_summary(label.rect);
            }
        } else {
            let label = ui.label(
                egui::RichText::new("StartingFrame : —")
                    .size(14.0 * scale)
                    .strong(),
            );
            include_summary(label.rect);
        }
        if let Some(frame1) = actual_timeleap_frame1_for_display(state) {
            let separator = ui.separator();
            include_summary(separator.rect);
            let label =
                ui.label(egui::RichText::new(format!("Frame1: {frame1}")).size(14.0 * scale));
            include_summary(label.rect);
            let copy = ui.button(texts.copy());
            include_summary(copy.rect);
            if copy.clicked() {
                ui.output_mut(|output| {
                    output.copied_text = frame1.to_string();
                });
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let checkbox = ui.checkbox(
                &mut state.table_follow_current,
                texts.timeline_follow_current(),
            );
            checkbox_rect = checkbox.rect;
        });
    });
    (summary_rect, checkbox_rect)
}

fn field_current_position_ui(ui: &mut egui::Ui, state: &mut FieldState, fps: f64, scale: f32) {
    if state.current_timeline_frame(fps).is_some() {
        if let Some(current_sfmt) = state.current_sfmt_frame(fps) {
            let current_size = egui::vec2(ui.available_width(), 24.0 * scale);
            ui.allocate_ui_with_layout(
                current_size,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new(format!("Current >> {current_sfmt}"))
                            .size(14.0 * scale),
                    ));
                },
            );
        }
    }
}
