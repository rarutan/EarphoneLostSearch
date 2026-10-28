use super::super::timeline::{field_text_scale, field_timeline_header_ui};
use super::*;

fn text_shape_rect(output: &egui::FullOutput, expected: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == expected => {
                Some(text.visual_bounding_rect())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("text shape {expected:?} was not rendered"))
}

fn text_shape_font_size(output: &egui::FullOutput, expected: &str) -> f32 {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == expected => text
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.font_id.size),
            _ => None,
        })
        .unwrap_or_else(|| panic!("text shape {expected:?} was not rendered"))
}

fn text_shape_rects(output: &egui::FullOutput, expected: &str) -> Vec<egui::Rect> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == expected => {
                Some(text.visual_bounding_rect())
            }
            _ => None,
        })
        .collect()
}

fn text_shape_color(output: &egui::FullOutput, expected: &str) -> egui::Color32 {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == expected => text
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.color),
            _ => None,
        })
        .unwrap_or_else(|| panic!("text color {expected:?} was not rendered"))
}

fn painted_rect_at_center(output: &egui::FullOutput, center: egui::Pos2) -> egui::Rect {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.rect.center().distance(center) < 0.5 => Some(rect.rect),
            _ => None,
        })
        .max_by(|left, right| {
            (left.width() * left.height()).total_cmp(&(right.width() * right.height()))
        })
        .unwrap_or_else(|| panic!("no painted rectangle found around {center:?}"))
}

fn text_shape_font_id(output: &egui::FullOutput, expected: &str) -> egui::FontId {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == expected => text
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.font_id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("text shape {expected:?} was not rendered"))
}

mod forms;
mod interactions;
mod main_ui;
mod misc;
mod table_style;
mod target_table;
