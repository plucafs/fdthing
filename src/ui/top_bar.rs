use eframe::egui;

use crate::app::FdGuiApp;
use crate::types::{FocusTarget, ResultView};

pub fn show(ctx: &egui::Context, app: &mut FdGuiApp) {
    let pattern_id = egui::Id::new("pattern_input");
    let ext_id = egui::Id::new("ext_textedit");
    let excl_id = egui::Id::new("exclude_input");

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
                    app.pattern.clear();
                    app.results.clear();
                    app.search_status = crate::types::SearchStatus::Idle;
                    app.error_message = None;
                    app.info_message = None;
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
            ui.horizontal(|ui| {
                ui.label("Ext:");
                let ext_resp = ui.add(
                    egui::TextEdit::singleline(&mut app.ext_filter)
                        .id(ext_id)
                        .hint_text("rs, py, txt…"),
                );
                if ext_resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    app.focus_after_search = Some(FocusTarget::Ext);
                    app.start_search();
                }
                ui.label("Exclude:");
                let excl_resp = ui.add(
                    egui::TextEdit::singleline(&mut app.exclude_filter)
                        .id(excl_id)
                        .hint_text("target, *.log…"),
                );
                if excl_resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    app.focus_after_search = Some(FocusTarget::Exclude);
                    app.start_search();
                }
            });

            // ── Deferred focus ───────────────────────────────────────────
            if let Some(ref target) = app.focus_after_search {
                let id = match target {
                    FocusTarget::Pattern => pattern_id,
                    FocusTarget::Ext => ext_id,
                    FocusTarget::Exclude => excl_id,
                };
                ui.memory_mut(|mem| mem.request_focus(id));
                app.focus_after_search = None;
            }
        });
    });
}
