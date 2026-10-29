use eframe::egui;

use crate::app::FdGuiApp;
use crate::types::{FocusTarget, ResultView};

pub fn show(ctx: &egui::Context, app: &mut FdGuiApp) {
    let pattern_id = egui::Id::new("pattern_input");
    egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
        ui.vertical(|ui| {
            // ── Row 0: pattern + action buttons ──────────────────────────
            ui.horizontal(|ui| {
                ui.label("🔍");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut app.pattern)
                        .id(pattern_id)
                        .hint_text("Search pattern (file name)…")
                        .desired_width(300.0),
                );

                let enter_pressed = response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));

                if ui.button("Search").clicked() || enter_pressed {
                    app.focus_after_search = Some(FocusTarget::Pattern);
                    app.start_search();
                }

                if matches!(app.search_status, crate::types::SearchStatus::Searching)
                    && ui.button("Stop").clicked()
                {
                    if let Some(ref flag) = app.cancel_flag {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    app.search_status = crate::types::SearchStatus::Done;
                    app.needs_sort = true;
                }

                if ui.button("Clear").clicked() {
                    app.clear_search();
                }
            });

            // ── Row 1: toggles + scale + theme + view ────────────────────
            ui.horizontal(|ui| {
                let mut opts_changed = false;
                opts_changed |=
                    ui.checkbox(&mut app.case_sensitive, "Aa case").changed();
                opts_changed |=
                    ui.checkbox(&mut app.use_regex, ".* regex").changed();
                opts_changed |=
                    ui.checkbox(&mut app.show_hidden, "Hidden").changed();
                opts_changed |=
                    ui.checkbox(&mut app.dirs_only, "Dirs").changed();
                opts_changed |=
                    ui.checkbox(&mut app.follow_symlinks, "Symlinks").changed();
                opts_changed |= ui
                    .checkbox(&mut app.respect_ignorefiles, ".gitignore")
                    .changed();
                if opts_changed {
                    app.save_config();
                }

                ui.separator();
                ui.label("UI Scale:");
                if ui
                    .add(
                        egui::DragValue::new(&mut app.ui_scale)
                            .speed(0.01)
                            .range(0.5..=3.0)
                            .max_decimals(2)
                            .suffix("×"),
                    )
                    .changed()
                {
                    ctx.set_zoom_factor(app.ui_scale);
                    app.save_config();
                }

                ui.separator();
                let theme_label = if app.light_mode { "Light" } else { "Dark" };
                egui::ComboBox::from_id_salt("theme_combo")
                    .selected_text(format!("Theme: {theme_label}"))
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut app.light_mode, false, "Dark")
                            .clicked()
                            || ui
                                .selectable_value(&mut app.light_mode, true, "Light")
                                .clicked()
                        {
                            app.save_config();
                        }
                    });

                ui.separator();
                let view_label = app.result_view.label();
                egui::ComboBox::from_id_salt("view_combo")
                    .selected_text(format!("View: {view_label}"))
                    .show_ui(ui, |ui| {
                        let mut changed = false;
                        changed |= ui
                            .selectable_value(
                                &mut app.result_view,
                                ResultView::Aligned,
                                ResultView::Aligned.label(),
                            )
                            .clicked();
                        changed |= ui
                            .selectable_value(
                                &mut app.result_view,
                                ResultView::Fluid,
                                ResultView::Fluid.label(),
                            )
                            .clicked();
                        changed |= ui
                            .selectable_value(
                                &mut app.result_view,
                                ResultView::Simple,
                                ResultView::Simple.label(),
                            )
                            .clicked();
                        if changed {
                            app.save_config();
                        }
                    });
            });

            // ── Row 2: Ext + Exclude ─────────────────────────────────────
            let mut filters_changed = false;
            ui.horizontal_wrapped(|ui| {
                filters_changed |= crate::ui::filter_chips::token_filter_ui(
                    ctx,
                    ui,
                    "extension_filter",
                    "Ext:",
                    &mut app.ext_tokens,
                    &mut app.ext_add_input,
                    &mut app.ext_filter_modal_open,
                    true,
                );
            });
            ui.horizontal_wrapped(|ui| {
                filters_changed |= crate::ui::filter_chips::token_filter_ui(
                    ctx,
                    ui,
                    "exclude_filter",
                    "Exclude:",
                    &mut app.exclude_tokens,
                    &mut app.exclude_add_input,
                    &mut app.exclude_filter_modal_open,
                    false,
            )});
            if filters_changed {
                app.save_config();
            }

            // ── Deferred focus ───────────────────────────────────────────
            if let Some(ref target) = app.focus_after_search {
                ui.memory_mut(|mem| mem.request_focus(pattern_id));
                if matches!(target, FocusTarget::Pattern) {
                    if app.move_cursor_to_end_on_focus {
                        move_cursor_to_end(ctx, pattern_id, &app.pattern);
                        app.move_cursor_to_end_on_focus = false;
                    } else if app.select_all_on_focus {
                        select_all(ctx, pattern_id, &app.pattern);
                        app.select_all_on_focus = false;
                    }
                }
                app.focus_after_search = None;
            }

            // ── Ctrl+F: focus pattern input and select all ──────────────
            if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
                ui.memory_mut(|mem| mem.request_focus(pattern_id));
                select_all(ctx, pattern_id, &app.pattern);
            }
        });
    });
}

fn select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    let end = text.chars().count();
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(end),
        )));
        state.store(ctx, id);
    }
}

fn move_cursor_to_end(ctx: &egui::Context, id: egui::Id, text: &str) {
    let end = text.chars().count();
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(end),
        )));
        state.store(ctx, id);
    }
}
