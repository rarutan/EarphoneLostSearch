use super::*;

#[test]
fn timeleap_window_viewport_is_kept_alive_on_other_main_tabs_only_when_open() {
    assert!(keep_timeleap_window_viewport(AppTab::Hatch, true));
    assert!(keep_timeleap_window_viewport(AppTab::NormalTimer, true));
    assert!(keep_timeleap_window_viewport(AppTab::Settings, true));
    assert!(!keep_timeleap_window_viewport(AppTab::Field, true));
    assert!(!keep_timeleap_window_viewport(AppTab::Hatch, false));
}

#[test]
fn keepalive_viewport_dispatches_target_search_request_to_app() {
    let mut app = UiApp::default();
    let context = egui::Context::default();
    let actions = crate::ui::field_view::FieldUiActions::default();
    app.dispatch_keepalive_window_actions(&context, actions);
    assert!(matches!(
        app.field.confirmed_target_search_form.status,
        crate::application::field::ConfirmedTargetSearchStatus::Idle
    ));

    let mut actions = crate::ui::field_view::FieldUiActions::default();
    actions.confirmed_target_search_requested = true;
    app.dispatch_keepalive_window_actions(&context, actions);
    assert!(matches!(
        app.field.confirmed_target_search_form.status,
        crate::application::field::ConfirmedTargetSearchStatus::Failed(_)
    ));
}

#[test]
fn keepalive_viewport_dispatches_unlocked_target_row_actions() {
    let mut app = UiApp::default();
    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 998,
        drift_frames: -2,
        grace_frames: 2,
        generation_start_frame: Some(900),
    };
    app.field.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::Results(vec![row.clone()]);
    assert!(!app.field.timeleap_timeline_adopted());

    let mut actions = crate::ui::field_view::FieldUiActions::default();
    actions.confirmed_target_search_row = Some(row.clone());
    app.dispatch_keepalive_window_actions(&egui::Context::default(), actions);
    assert_eq!(app.field.committed_target_frame(), Some(998));

    app.field.encounter_offset = "20".into();
    let mut actions = crate::ui::field_view::FieldUiActions::default();
    actions.confirmed_target_offset_drift_frames = Some(-2);
    app.dispatch_keepalive_window_actions(&egui::Context::default(), actions);
    assert_eq!(app.field.encounter_offset, "22");
    assert!(app.field.use_offset);
}
