use crate::domain::rng::MAX_SFMT_FRAME;
use crate::ui::i18n::Language;
use eframe::egui;
use serde::{Deserialize, Serialize};

const FLASH_SHORT_JA: &str = "短い点滅";
const FLASH_LONG_JA: &str = "長い点滅";
const FLASH_STATIC_JA: &str = "固定強調";
const FLASH_SHORT_EN: &str = "Short flash";
const FLASH_LONG_EN: &str = "Long flash";
const FLASH_STATIC_EN: &str = "Static highlight";
const FLASH_SHORT_MS: u64 = 180;
const FLASH_LONG_MS: u64 = 650;
const FLASH_STATIC_MS: u64 = 1_500;
const COLOR_STANDARD_JA: &str = "標準";
const COLOR_HIGH_CONTRAST_JA: &str = "高コントラスト";
const COLOR_MONOCHROME_JA: &str = "モノクロ";
const COLOR_STANDARD_EN: &str = "Standard";
const COLOR_HIGH_CONTRAST_EN: &str = "High contrast";
const COLOR_MONOCHROME_EN: &str = "Monochrome";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ColorScheme {
    Standard,
    HighContrast,
    Monochrome,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum FlashMode {
    Short,
    Long,
    Static,
}

impl Default for FlashMode {
    fn default() -> Self {
        Self::Short
    }
}

impl FlashMode {
    pub(crate) fn label(self, language: Language) -> &'static str {
        match (language, self) {
            (Language::Japanese, Self::Short) => FLASH_SHORT_JA,
            (Language::Japanese, Self::Long) => FLASH_LONG_JA,
            (Language::Japanese, Self::Static) => FLASH_STATIC_JA,
            (Language::English, Self::Short) => FLASH_SHORT_EN,
            (Language::English, Self::Long) => FLASH_LONG_EN,
            (Language::English, Self::Static) => FLASH_STATIC_EN,
        }
    }

    pub(crate) fn duration_millis(self) -> u64 {
        match self {
            Self::Short => FLASH_SHORT_MS,
            Self::Long => FLASH_LONG_MS,
            Self::Static => FLASH_STATIC_MS,
        }
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::Standard
    }
}

impl ColorScheme {
    pub(crate) fn label(self, language: Language) -> &'static str {
        match (language, self) {
            (Language::Japanese, Self::Standard) => COLOR_STANDARD_JA,
            (Language::Japanese, Self::HighContrast) => COLOR_HIGH_CONTRAST_JA,
            (Language::Japanese, Self::Monochrome) => COLOR_MONOCHROME_JA,
            (Language::English, Self::Standard) => COLOR_STANDARD_EN,
            (Language::English, Self::HighContrast) => COLOR_HIGH_CONTRAST_EN,
            (Language::English, Self::Monochrome) => COLOR_MONOCHROME_EN,
        }
    }

    pub(crate) fn colors_for_ui(self, ui: &egui::Ui) -> PaletteColors {
        self.colors_for(ui.visuals().dark_mode)
    }

    /// 配色の意味は維持したまま、標準の背景・文字色はOSのライト／
    /// ダークテーマに合わせる。カスタム配色から標準へ戻すときも、
    /// 現在のテーマ用の値を再生成できるようにする。
    pub(crate) fn colors_for(self, dark_mode: bool) -> PaletteColors {
        if !dark_mode {
            return match self {
                Self::Standard => PaletteColors {
                    text: egui::Color32::from_gray(32),
                    timer_text: egui::Color32::from_rgb(0, 114, 178),
                    target_yes: egui::Color32::from_rgb(0, 114, 178),
                    target_no: egui::Color32::from_rgb(213, 94, 0),
                    current_row: egui::Color32::from_rgb(190, 225, 245),
                    flash: egui::Color32::from_rgb(180, 140, 0),
                    error: egui::Color32::from_rgb(170, 40, 110),
                    neutral: egui::Color32::from_gray(110),
                    panel_fill: egui::Color32::from_gray(248),
                    window_fill: egui::Color32::from_gray(248),
                    extreme_bg: egui::Color32::WHITE,
                    faint_bg: egui::Color32::from_rgb(238, 238, 238),
                },
                Self::HighContrast => PaletteColors {
                    text: egui::Color32::BLACK,
                    timer_text: egui::Color32::from_rgb(0, 82, 136),
                    target_yes: egui::Color32::from_rgb(0, 82, 136),
                    target_no: egui::Color32::from_rgb(170, 62, 0),
                    current_row: egui::Color32::from_rgb(155, 205, 235),
                    flash: egui::Color32::from_rgb(130, 100, 0),
                    error: egui::Color32::from_rgb(145, 20, 85),
                    neutral: egui::Color32::from_gray(80),
                    panel_fill: egui::Color32::WHITE,
                    window_fill: egui::Color32::WHITE,
                    extreme_bg: egui::Color32::WHITE,
                    faint_bg: egui::Color32::from_rgb(230, 230, 230),
                },
                Self::Monochrome => PaletteColors {
                    text: egui::Color32::BLACK,
                    timer_text: egui::Color32::from_gray(20),
                    target_yes: egui::Color32::from_gray(20),
                    target_no: egui::Color32::from_gray(80),
                    current_row: egui::Color32::from_gray(205),
                    flash: egui::Color32::BLACK,
                    error: egui::Color32::from_gray(60),
                    neutral: egui::Color32::from_gray(110),
                    panel_fill: egui::Color32::from_gray(245),
                    window_fill: egui::Color32::WHITE,
                    extreme_bg: egui::Color32::WHITE,
                    faint_bg: egui::Color32::from_gray(232),
                },
            };
        }
        match self {
            Self::Standard => PaletteColors {
                text: egui::Color32::from_gray(235),
                timer_text: egui::Color32::from_rgb(143, 211, 255),
                target_yes: egui::Color32::from_rgb(86, 180, 233),
                target_no: egui::Color32::from_rgb(230, 159, 0),
                current_row: egui::Color32::from_rgb(37, 112, 153),
                flash: egui::Color32::from_rgb(240, 228, 66),
                error: egui::Color32::from_rgb(204, 121, 167),
                neutral: egui::Color32::from_gray(180),
                panel_fill: egui::Color32::from_rgb(20, 25, 34),
                window_fill: egui::Color32::from_rgb(28, 34, 46),
                extreme_bg: egui::Color32::from_rgb(13, 17, 24),
                faint_bg: egui::Color32::from_rgb(32, 40, 53),
            },
            Self::HighContrast => PaletteColors {
                text: egui::Color32::WHITE,
                timer_text: egui::Color32::from_rgb(125, 211, 255),
                target_yes: egui::Color32::from_rgb(0, 191, 255),
                target_no: egui::Color32::from_rgb(255, 127, 14),
                current_row: egui::Color32::from_rgb(0, 114, 178),
                flash: egui::Color32::from_rgb(255, 241, 0),
                error: egui::Color32::from_rgb(255, 105, 180),
                neutral: egui::Color32::from_gray(230),
                panel_fill: egui::Color32::from_rgb(8, 12, 20),
                window_fill: egui::Color32::from_rgb(15, 22, 35),
                extreme_bg: egui::Color32::BLACK,
                faint_bg: egui::Color32::from_rgb(30, 42, 58),
            },
            Self::Monochrome => PaletteColors {
                text: egui::Color32::WHITE,
                timer_text: egui::Color32::WHITE,
                target_yes: egui::Color32::WHITE,
                target_no: egui::Color32::from_gray(190),
                current_row: egui::Color32::from_gray(82),
                flash: egui::Color32::WHITE,
                error: egui::Color32::from_gray(220),
                neutral: egui::Color32::from_gray(180),
                panel_fill: egui::Color32::from_gray(18),
                window_fill: egui::Color32::from_gray(28),
                extreme_bg: egui::Color32::from_gray(8),
                faint_bg: egui::Color32::from_gray(42),
            },
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PaletteColors {
    pub(crate) text: egui::Color32,
    pub(crate) timer_text: egui::Color32,
    pub(crate) target_yes: egui::Color32,
    pub(crate) target_no: egui::Color32,
    pub(crate) current_row: egui::Color32,
    pub(crate) flash: egui::Color32,
    pub(crate) error: egui::Color32,
    pub(crate) neutral: egui::Color32,
    pub(crate) panel_fill: egui::Color32,
    pub(crate) window_fill: egui::Color32,
    pub(crate) extreme_bg: egui::Color32,
    pub(crate) faint_bg: egui::Color32,
}

pub(crate) fn draw_timer_flash(
    ui: &egui::Ui,
    rect: egui::Rect,
    flashing: bool,
    scheme: ColorScheme,
) {
    if !flashing {
        return;
    }
    ui.painter().rect_stroke(
        rect.shrink(3.0),
        8.0,
        egui::Stroke::new(4.0_f32, scheme.colors_for_ui(ui).flash),
    );
}

pub(crate) fn wide_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> bool {
    // 固定枠の内側に少し余白を残し、ボタンを水平方向の中央へ配置する。
    // `add_sized`だけでは左端を起点にするため、枠線との接触と右側の余白が生じる。
    let width = (ui.available_width() - 4.0).max(0.0);
    let mut clicked = false;
    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        clicked = ui
            .add_enabled_ui(enabled, |ui| {
                ui.add_sized([width, 30.0], egui::Button::new(label))
                    .clicked()
            })
            .inner;
    });
    clicked
}

/// 瞬きタブの実測Frame入力の隣に置く小型アクションボタンのサイズ。
/// 両タブで同じ寸法を使い、レイアウトのずれを防ぐ。
pub(crate) const COMPACT_ACTION_SIZE: [f32; 2] = [90.0, 30.0];

/// 親レイアウトが確保した寸法のままグループ枠を描画する。
///
/// `Frame::show`は子要素が収まらない場合に枠を拡張するため、条件行の多い
/// フィールド側だけ外枠が高くなることがある。枠を個別に描画し、登録しない
/// 子UIへ内容を置くことで、内部の通常レイアウトを保ったまま外枠を固定する。
pub(crate) fn fixed_group_panel(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let outer_rect = ui.max_rect();
    let frame = egui::Frame::group(ui.style());
    let margin = frame.inner_margin;
    let inner_rect = egui::Rect::from_min_max(
        egui::pos2(
            outer_rect.left() + margin.left,
            outer_rect.top() + margin.top,
        ),
        egui::pos2(
            outer_rect.right() - margin.right,
            outer_rect.bottom() - margin.bottom,
        ),
    );

    // 親から割り当てられた矩形と同じ領域を子UIへ渡す。
    ui.set_min_size(outer_rect.size());
    ui.painter().add(frame.paint(outer_rect));

    let mut content_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    content_ui.set_clip_rect(inner_rect);
    add_contents(&mut content_ui);
}

pub(crate) fn frame_spinner(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    step: i64,
    wheel_enabled: bool,
) -> bool {
    frame_spinner_with_commit(ui, value, width, step, wheel_enabled).0
}

/// 編集確定後に反応するFrame入力の状態。
/// `committed`はフォーカス移動、Enter、またはステップボタン操作で立つ。
pub(crate) fn frame_spinner_with_commit(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    step: i64,
    wheel_enabled: bool,
) -> (bool, bool) {
    let enabled = ui.is_enabled();
    let mut changed = false;
    let mut committed = false;
    let mut hovered = false;
    let frame = egui::Frame::none()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .rounding(ui.visuals().widgets.inactive.rounding)
        .inner_margin(egui::Margin::same(0.0));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        ui.horizontal(|ui| {
            let original = value.clone();
            let mut edited = original.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut edited)
                    .id_salt(value as *const String)
                    .desired_width(width)
                    .frame(false)
                    .horizontal_align(egui::Align::Max),
            );
            hovered |= response.hovered() || response.has_focus();
            committed |=
                response.lost_focus() || ui.input(|input| input.key_pressed(egui::Key::Enter));
            if response.changed() {
                // TextEditへ渡した文字列をその場で削除すると、カーソル位置が
                // 揺れて入力欄がちらつく。無効な編集は値へ反映せず破棄する。
                // ボタンの増減は引き続き通常どおり受け付ける。
                if is_valid_signed_integer_text(&edited) {
                    *value = edited;
                    changed = true;
                }
            }
            // 2つのステップボタンを入力欄の高さ内へ収める。アプリ全体の操作高さは
            // コンパクトな数値欄より大きいため、ここでは入力欄に合わせて再計算する。
            let input_height = response.rect.height().max(ui.spacing().interact_size.y);
            let arrow_height = (input_height / 2.0).max(1.0);
            let arrow_width = 16.0;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().button_padding = egui::vec2(0.0, 0.0);
                ui.spacing_mut().interact_size = egui::vec2(arrow_width, arrow_height);
                ui.vertical(|ui| {
                    let up = ui.add_sized(
                        [arrow_width, arrow_height],
                        egui::Button::new(
                            egui::RichText::new("▲").size((arrow_height * 0.55).max(6.0)),
                        )
                        .small(),
                    );
                    hovered |= up.hovered();
                    if enabled && up.clicked() {
                        adjust_frame_text(value, step);
                        changed = true;
                        committed = true;
                    }
                    let down = ui.add_sized(
                        [arrow_width, arrow_height],
                        egui::Button::new(
                            egui::RichText::new("▼").size((arrow_height * 0.55).max(6.0)),
                        )
                        .small(),
                    );
                    hovered |= down.hovered();
                    if enabled && down.clicked() {
                        adjust_frame_text(value, -step);
                        changed = true;
                        committed = true;
                    }
                });
            });
        });
    });
    if enabled && hovered {
        if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            adjust_frame_text(value, step);
            changed = true;
            committed = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
            adjust_frame_text(value, -step);
            changed = true;
            committed = true;
        }
        if wheel_enabled {
            let wheel = ui.input(|input| input.raw_scroll_delta.y);
            if wheel != 0.0 {
                // egui reports the content movement: positive Y means the
                // content moves down (physical wheel-down). Numeric controls
                // follow the conventional up=increase, down=decrease rule.
                adjust_frame_text(value, if wheel > 0.0 { -step } else { step });
                changed = true;
                committed = true;
                ui.input_mut(|input| {
                    input.raw_scroll_delta.y = 0.0;
                    input.smooth_scroll_delta.y = 0.0;
                });
            }
        }
    }
    (changed, committed)
}

/// オフセット専用スピナー。入力途中の奇数は保持して`10`のような値を
/// 入力できるようにし、確定時に最寄りの偶数へ丸める。範囲外の数値は
/// 入力された時点で範囲端へ寄せるため、画面表示と計算側の制約を一致させる。
pub(crate) fn offset_spinner_with_commit(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    step: i64,
    wheel_enabled: bool,
    minimum: i64,
    maximum: i64,
) -> (bool, bool) {
    let original = value.clone();
    let (mut changed, committed) = frame_spinner_with_commit(ui, value, width, step, wheel_enabled);
    let trimmed = value.trim();
    let Some(number) = trimmed.parse::<i64>().ok() else {
        // 空文字は編集中の状態として保持する。単独のマイナス記号など、確定値に
        // 変換できない途中入力は確定時に直前の値へ戻す。
        if committed && !trimmed.is_empty() {
            *value = original;
            changed = false;
        }
        return (changed, committed);
    };

    let bounded = number.clamp(minimum, maximum);
    if bounded != number {
        *value = bounded.to_string();
        changed = true;
        ui.ctx().request_repaint();
        return (changed, committed);
    }
    if committed && bounded % 2 != 0 {
        *value = bounded.saturating_sub(bounded.signum()).to_string();
        changed = true;
        ui.ctx().request_repaint();
    }
    (changed, committed)
}

/// 0以上のFrame専用スピナー。TargetFrameなど、負値を許可しない欄で使う。
pub(crate) fn nonnegative_frame_spinner(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    step: i64,
    wheel_enabled: bool,
) -> bool {
    nonnegative_frame_spinner_with_commit(ui, value, width, step, wheel_enabled).0
}

pub(crate) fn nonnegative_frame_spinner_with_commit(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    step: i64,
    wheel_enabled: bool,
) -> (bool, bool) {
    let original = value.clone();
    let (changed, committed) = frame_spinner_with_commit(ui, value, width, step, wheel_enabled);
    let trimmed = value.trim();
    let invalid = !trimmed.is_empty()
        && trimmed
            .parse::<i64>()
            .map(|number| number < 0)
            .unwrap_or(true);
    if changed && invalid {
        *value = original;
        return (false, committed);
    }
    (changed, committed)
}

/// NPC数用の上下スピナー。空欄は入力途中として許容し、確定値は0～50に制限する。
pub(crate) fn npc_spinner(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    wheel_enabled: bool,
) -> bool {
    let original = value.clone();
    let changed = frame_spinner(ui, value, width, 1, wheel_enabled);
    let trimmed = value.trim();
    let invalid = !trimmed.is_empty()
        && trimmed
            .parse::<i64>()
            .map(|number| !(0..=50).contains(&number))
            .unwrap_or(true);
    if changed && invalid {
        *value = original;
        return false;
    }
    changed
}

/// 消費数用の上下スピナー。NPC数とは異なり、補正消費は50を上限にせず
/// SFMT位置の上限まで入力できる。
pub(crate) fn consumption_spinner(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    wheel_enabled: bool,
) -> bool {
    let original = value.clone();
    let changed = frame_spinner(ui, value, width, 1, wheel_enabled);
    let trimmed = value.trim();
    let invalid = !trimmed.is_empty()
        && trimmed
            .parse::<i64>()
            .map(|number| !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&number))
            .unwrap_or(true);
    if changed && invalid {
        *value = original;
        return false;
    }
    changed
}

/// 補正用の横向きFrameスピナー。表示は`← 0F →`とし、既存の
/// キーボード上下・マウスホイール操作も縦向きスピナーと共有する。
pub(crate) fn horizontal_frame_spinner(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    step: i64,
    wheel_enabled: bool,
) -> bool {
    let enabled = ui.is_enabled();
    let mut changed = false;
    let mut hovered = false;
    let frame = egui::Frame::none()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .rounding(ui.visuals().widgets.inactive.rounding)
        .inner_margin(egui::Margin::same(0.0));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            let left = ui.add_enabled(enabled, egui::Button::new("←").small());
            hovered |= left.hovered();
            if left.clicked() {
                adjust_frame_text(value, -step);
                changed = true;
            }
            let original = value.clone();
            let mut edited = original.clone();
            let mut response = ui.add(
                egui::TextEdit::singleline(&mut edited)
                    .id_salt(value as *const String)
                    .desired_width(width)
                    .frame(false)
                    .horizontal_align(egui::Align::Center),
            );
            hovered |= response.hovered() || response.has_focus();
            if response.changed() {
                if is_valid_signed_integer_text(&edited) {
                    *value = edited;
                    changed = true;
                } else {
                    *value = original;
                    response.changed = false;
                }
            }
            ui.label("F");
            let right = ui.add_enabled(enabled, egui::Button::new("→").small());
            hovered |= right.hovered();
            if right.clicked() {
                adjust_frame_text(value, step);
                changed = true;
            }
        });
    });
    if enabled && hovered {
        if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            adjust_frame_text(value, step);
            changed = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
            adjust_frame_text(value, -step);
            changed = true;
        }
        if wheel_enabled {
            let wheel = ui.input(|input| input.raw_scroll_delta.y);
            if wheel != 0.0 {
                // Keep wheel direction consistent with the vertical spinner:
                // scrolling up increases and scrolling down decreases.
                adjust_frame_text(value, if wheel > 0.0 { -step } else { step });
                changed = true;
                ui.input_mut(|input| {
                    input.raw_scroll_delta.y = 0.0;
                    input.smooth_scroll_delta.y = 0.0;
                });
            }
        }
    }
    changed
}

pub(crate) fn install_app_style(ctx: &egui::Context, scheme: ColorScheme) {
    // eguiのライト／ダーク両テーマを設定する。OSテーマが起動後に通知されても
    // レイアウトを失わず、別テーマの古い黒背景へ戻らないようにする。
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.window_margin = egui::Margin::same(14.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size = egui::vec2(40.0, 28.0);
        apply_color_visuals(&mut style.visuals, scheme);
        style.visuals.window_rounding = egui::Rounding::same(10.0);
        style.visuals.menu_rounding = egui::Rounding::same(8.0);
    });
}

/// 配色変更時に色だけを更新する。spacingやtext_stylesを再構築しないため、
/// 配色を切り替えても文字サイズ・余白・入力欄の寸法は変化しない。
pub(crate) fn apply_color_scheme(ctx: &egui::Context, scheme: ColorScheme) {
    // ライト／ダーク両方のスタイルへ適用し、OSテーマ切替後に別配色の背景が
    // 復活しないようにする。
    ctx.all_styles_mut(|style| apply_color_visuals(&mut style.visuals, scheme));
}

fn apply_color_visuals(visuals: &mut egui::Visuals, scheme: ColorScheme) {
    let colors = scheme.colors_for(visuals.dark_mode);
    visuals.panel_fill = colors.panel_fill;
    visuals.window_fill = colors.window_fill;
    visuals.extreme_bg_color = colors.extreme_bg;
    visuals.faint_bg_color = colors.faint_bg;
    visuals.code_bg_color = colors.extreme_bg;
    visuals.selection.bg_fill = colors.current_row;
    visuals.selection.stroke.color = colors.timer_text;
    visuals.hyperlink_color = colors.timer_text;
    visuals.warn_fg_color = colors.target_no;
    visuals.error_fg_color = colors.error;
    visuals.override_text_color = Some(colors.text);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.fg_stroke.color = colors.text;
    }
}

/// 数値入力欄。入力途中の空文字は許容しつつ、確定値に使えない編集は
/// 値へ反映しない。`allow_decimal` はFPSや秒間隔、`allow_range`
/// はNPCの範囲入力（例: `10~20`）にだけ使う。
pub(crate) fn numeric_text_edit(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    allow_decimal: bool,
    allow_range: bool,
) -> egui::Response {
    let original = value.clone();
    let mut edited = original.clone();
    let mut response = ui.add(
        egui::TextEdit::singleline(&mut edited)
            .id_salt(value as *const String)
            .desired_width(width)
            .horizontal_align(egui::Align::Max),
    );
    if response.changed() {
        if is_valid_numeric_text(&edited, allow_decimal, allow_range) {
            *value = edited;
        } else {
            // 無効文字を削除して再描画するのではなく、編集イベント自体を
            // 取り消す。これにより入力中のカーソル／表示がちらつかない。
            *value = original;
            response.changed = false;
        }
    }
    response
}

fn is_valid_numeric_text(value: &str, allow_decimal: bool, allow_range: bool) -> bool {
    let mut decimal_seen = false;
    let mut range_seen = false;
    let syntax_ok = value.chars().all(|character| {
        if character.is_ascii_digit() {
            true
        } else if allow_decimal && character == '.' && !decimal_seen {
            decimal_seen = true;
            true
        } else if allow_range && character == '~' && !range_seen {
            range_seen = true;
            true
        } else {
            false
        }
    });
    if !syntax_ok || allow_decimal || allow_range || value.is_empty() {
        return syntax_ok;
    }
    value
        .parse::<i64>()
        .map(|number| (0..=MAX_SFMT_FRAME).contains(&number))
        .unwrap_or(false)
}

fn is_valid_signed_integer_text(value: &str) -> bool {
    let syntax_ok = value
        .chars()
        .enumerate()
        .all(|(index, character)| character.is_ascii_digit() || (character == '-' && index == 0));
    if !syntax_ok || value.is_empty() || value == "-" {
        return syntax_ok;
    }
    value
        .parse::<i64>()
        .map(|number| number.unsigned_abs() <= MAX_SFMT_FRAME as u64)
        .unwrap_or(false)
}

pub(crate) fn adjust_frame_text(value: &mut String, delta: i64) {
    if let Ok(current) = value.trim().parse::<i64>() {
        *value = current.saturating_add(delta).to_string();
    }
}

pub(crate) fn install_japanese_font(ctx: &egui::Context) {
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Windows\Fonts\meiryo.ttc",
            r"C:\Windows\Fonts\YuGothM.ttc",
            r"C:\Windows\Fonts\msgothic.ttc",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
            "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
        ]
    };
    let Some(path) = candidates
        .iter()
        .find(|path| std::path::Path::new(path).exists())
    else {
        return;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("japanese".into(), egui::FontData::from_owned(bytes));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.insert(0, "japanese".into());
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::{apply_color_scheme, ColorScheme};

    #[test]
    fn changing_color_scheme_preserves_layout_and_text_sizes() {
        let ctx = eframe::egui::Context::default();
        let before = (*ctx.style()).clone();

        apply_color_scheme(&ctx, ColorScheme::HighContrast);
        let high_contrast = (*ctx.style()).clone();
        assert_eq!(high_contrast.spacing, before.spacing);
        assert_eq!(high_contrast.text_styles, before.text_styles);

        apply_color_scheme(&ctx, ColorScheme::Standard);
        let restored = (*ctx.style()).clone();
        assert_eq!(restored.spacing, before.spacing);
        assert_eq!(restored.text_styles, before.text_styles);
    }

    #[test]
    fn standard_scheme_restores_a_light_system_background() {
        let ctx = eframe::egui::Context::default();
        ctx.set_style(eframe::egui::Theme::Light.default_style());
        let light = eframe::egui::Theme::Light.default_visuals();

        apply_color_scheme(&ctx, ColorScheme::HighContrast);
        apply_color_scheme(&ctx, ColorScheme::Standard);

        let restored = (*ctx.style()).clone();
        assert!(!restored.visuals.dark_mode);
        assert_eq!(restored.visuals.panel_fill, light.panel_fill);
        assert_eq!(restored.visuals.window_fill, light.window_fill);
        assert_eq!(restored.visuals.extreme_bg_color, light.extreme_bg_color);
    }
}
