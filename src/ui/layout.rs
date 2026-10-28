//! 複数ビューが共有する固定レイアウト寸法。
//!
//! 表示内容や計算用の定数は各ビュー／アプリケーション層に残し、矩形配置に使う
//! 共通寸法だけをここで管理する。

pub(crate) const TOP_PANEL_HEIGHT: f32 = 220.0;
pub(crate) const LEFT_PANEL_WIDTH: f32 = 430.0;
pub(crate) const PANEL_GAP: f32 = 8.0;
pub(crate) const PANEL_PADDING: f32 = 8.0;
pub(crate) const TIMER_PANEL_HEIGHT: f32 = 84.0;
pub(crate) const ACTION_ROW_HEIGHT: f32 = 32.0;
pub(crate) const MIN_WINDOW_WIDTH: f32 = 420.0;
pub(crate) const MIN_WINDOW_HEIGHT: f32 = 120.0;

/// 親が確保した矩形を左右2枠に分け、指定幅・間隔を親の範囲内へ収める。
pub(crate) fn split_panel_rects(
    parent: eframe::egui::Rect,
    preferred_left_width: f32,
    preferred_gap: f32,
) -> (eframe::egui::Rect, eframe::egui::Rect) {
    let gap = preferred_gap.max(0.0).min(parent.width());
    let left_width = preferred_left_width
        .max(0.0)
        .min((parent.width() - gap).max(0.0));
    let left = eframe::egui::Rect::from_min_size(
        parent.min,
        eframe::egui::vec2(left_width, parent.height()),
    );
    let right_left = (left.right() + gap).min(parent.right());
    let right = eframe::egui::Rect::from_min_max(
        eframe::egui::pos2(right_left, parent.top()),
        parent.right_bottom(),
    );
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::split_panel_rects;
    use eframe::egui::{pos2, vec2, Rect};

    #[test]
    fn split_panel_rects_stay_inside_assigned_parent_at_multiple_widths() {
        for width in [0.0, 420.0, 1_052.0, 1_600.0] {
            let parent = Rect::from_min_size(pos2(17.0, 31.0), vec2(width, 240.0));
            let (left, right) = split_panel_rects(parent, 430.0, 8.0);

            assert!(left.left() >= parent.left());
            assert!(left.right() <= parent.right());
            assert!(right.left() >= left.right());
            assert!(right.right() <= parent.right());
            assert_eq!(left.top(), parent.top());
            assert_eq!(right.bottom(), parent.bottom());
        }
    }
}
