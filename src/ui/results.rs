use std::path::PathBuf;

use eframe::egui;
use egui::{Color32, CursorIcon, ScrollArea, Sense};

use crate::app::FdGuiApp;
use crate::types::{format_date, format_size, ResultView, SortOrder};

pub fn show(ctx: &egui::Context, app: &mut FdGuiApp) {
    egui::CentralPanel::default().show(ctx, |ui| {
        // ── Status + Sort ────────────────────────────────────────────────
        ui.horizontal(|ui| {
            match app.search_status {
                crate::types::SearchStatus::Idle => {
                    ui.label("Ready — type a pattern and press Search");
                }
                crate::types::SearchStatus::Searching => {
                    ui.label(format!(
                        "Searching… {} matches so far",
                        app.results.len()
                    ));
                }
                crate::types::SearchStatus::Done => {
                    ui.label(format!(
                        "Done — {} matches found",
                        app.results.len()
                    ));
                }
            }
            ui.separator();
            ui.label("Sort:");
            let mut sort_order = app.sort_order;
            let mut changed = false;
            egui::ComboBox::from_id_salt("sort_combo")
                .selected_text(sort_order.label())
                .show_ui(ui, |ui| {
                    for order in SortOrder::ALL {
                        if ui
                            .selectable_value(&mut sort_order, order, order.label())
                            .changed()
                        {
                            changed = true;
                        }
                    }
                });
            if changed {
                app.sort_order = sort_order;
                app.needs_sort = true;
                app.save_config();
            }
        });

        // ── Messages ─────────────────────────────────────────────────────
        if let Some(ref err) = app.error_message {
            egui::CollapsingHeader::new(
                egui::RichText::new("Error").color(Color32::RED),
            )
            .default_open(true)
            .show(ui, |ui| {
                ui.colored_label(Color32::RED, err);
            });
        }
        if let Some(ref info) = app.info_message {
            egui::CollapsingHeader::new(
                egui::RichText::new("Info").color(Color32::GRAY),
            )
            .default_open(true)
            .show(ui, |ui| {
                ui.colored_label(Color32::GRAY, info);
            });
        }

        ui.separator();

        // ── Column setup ─────────────────────────────────────────────────
        let name_w = 260.0;
        let path_w = 400.0;
        let size_w = 80.0;
        let date_w = 140.0;
        let separator_w = 15.0;
        let row_w = name_w + path_w + size_w + date_w + separator_w * 3.0 + 20.0;

        // Column headers (only for table views)
        if app.result_view != ResultView::Simple {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(name_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Name").selectable(false));
                ui.separator();
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(path_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Path").selectable(false));
                ui.separator();
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(size_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Size").selectable(false));
                ui.separator();
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(date_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Modified").selectable(false));
            });
            ui.separator();
        }

        let mut open_path: Option<PathBuf> = None;
        let mut open_parent: Option<PathBuf> = None;

        // Results
        match app.result_view {
            ResultView::Aligned => {
                ui.set_min_width(row_w);
                ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.set_min_width(row_w);
                        render_rows(
                            ui,
                            &app.results,
                            name_w,
                            path_w,
                            size_w,
                            date_w,
                            true,
                            &mut open_path,
                            &mut open_parent,
                        );
                    });
            }
            ResultView::Fluid => {
                ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let limit = 10_000;
                        for entry in app.results.iter().take(limit) {
                            let icon = if entry.is_dir { "📁" } else { "📄" };
                            let file_name = entry
                                .path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("???");
                            let parent = entry.path.parent().and_then(|p| p.to_str()).unwrap_or("");
                            let size_str = if entry.is_dir {
                                "—".into()
                            } else {
                                format_size(entry.size)
                            };
                            let date_str = format_date(entry.modified);

                            ui.horizontal(|ui| {
                                let name = format!("{icon} {file_name}");
                                let name_resp = ui.add(
                                    egui::Label::new(name)
                                        .sense(Sense::click())
                                        .selectable(false),
                                );
                                ui.separator();
                                ui.add(
                                    egui::Label::new(parent)
                                        .selectable(false),
                                );
                                ui.separator();
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&size_str)
                                            .monospace(),
                                    )
                                    .selectable(false),
                                );
                                ui.separator();
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&date_str)
                                            .monospace(),
                                    )
                                    .selectable(false),
                                );

                                let name_resp = name_resp
                                    .on_hover_cursor(CursorIcon::PointingHand);
                                if name_resp.clicked() {
                                    open_path = Some(entry.path.clone());
                                }
                                if name_resp.secondary_clicked() {
                                    open_parent = entry
                                        .path
                                        .parent()
                                        .map(|p| p.to_path_buf());
                                }
                            });
                        }
                        if app.results.len() > limit {
                            ui.colored_label(
                                Color32::YELLOW,
                                format!(
                                    "… and {} more",
                                    app.results.len() - limit
                                ),
                            );
                        }
                    });
            }
            ResultView::Simple => {
                ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let limit = 10_000;
                        for entry in app.results.iter().take(limit) {
                            let icon =
                                if entry.is_dir { "📁" } else { "📄" };
                            let text = format!(
                                "{icon} {}",
                                entry.path.display()
                            );
                            let resp = ui
                                .add(
                                    egui::Label::new(text)
                                        .truncate()
                                        .sense(Sense::click()),
                                )
                                .on_hover_cursor(CursorIcon::PointingHand);
                            if resp.clicked() {
                                open_path = Some(entry.path.clone());
                            }
                            if resp.secondary_clicked() {
                                open_parent = entry
                                    .path
                                    .parent()
                                    .map(|p| p.to_path_buf());
                            }
                        }
                        if app.results.len() > limit {
                            ui.colored_label(
                                Color32::YELLOW,
                                format!(
                                    "… and {} more",
                                    app.results.len() - limit
                                ),
                            );
                        }
                    });
            }
        }

        // ── Open handlers ────────────────────────────────────────────────
        if let Some(p) = open_path {
            match open::that(&p) {
                Ok(()) => {
                    app.info_message =
                        Some(format!("Opened {}", p.display()));
                }
                Err(e) => {
                    app.error_message =
                        Some(format!("Cannot open {}: {e}", p.display()));
                }
            }
        }
        if let Some(p) = open_parent {
            match open::that(&p) {
                Ok(()) => {
                    app.info_message =
                        Some(format!("Opened folder {}", p.display()));
                }
                Err(e) => {
                    app.error_message =
                        Some(format!("Cannot open {}: {e}", p.display()));
                }
            }
        }
    });
}

fn render_rows(
    ui: &mut egui::Ui,
    results: &[crate::types::ResultEntry],
    name_w: f32,
    path_w: f32,
    size_w: f32,
    date_w: f32,
    truncate: bool,
    open_path: &mut Option<PathBuf>,
    open_parent: &mut Option<PathBuf>,
) {
    let limit = 10_000;
    for entry in results.iter().take(limit) {
        let icon = if entry.is_dir { "📁" } else { "📄" };
        let file_name = entry
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("???");
        let parent = entry.path.parent().and_then(|p| p.to_str()).unwrap_or("");
        let size_str = if entry.is_dir {
            "—".into()
        } else {
            format_size(entry.size)
        };
        let date_str = format_date(entry.modified);

        ui.horizontal(|ui| {
            let name = format!("{icon} {file_name}");
            let mut name_label = egui::Label::new(name)
                .sense(Sense::click())
                .selectable(false);
            if truncate {
                name_label = name_label.truncate();
            }
            let (name_rect, _) = ui.allocate_exact_size(
                egui::vec2(name_w, 0.0),
                Sense::hover(),
            );
            let name_resp = ui.put(name_rect, name_label);
            ui.separator();

            let mut path_label =
                egui::Label::new(parent).selectable(false);
            if truncate {
                path_label = path_label.truncate();
            }
            let (path_rect, _) = ui.allocate_exact_size(
                egui::vec2(path_w, 0.0),
                Sense::hover(),
            );
            ui.put(path_rect, path_label);
            ui.separator();

            let (size_rect, _) = ui.allocate_exact_size(
                egui::vec2(size_w, 0.0),
                Sense::hover(),
            );
            ui.put(
                size_rect,
                egui::Label::new(
                    egui::RichText::new(&size_str).monospace(),
                )
                .selectable(false),
            );
            ui.separator();

            let (date_rect, _) = ui.allocate_exact_size(
                egui::vec2(date_w, 0.0),
                Sense::hover(),
            );
            ui.put(
                date_rect,
                egui::Label::new(
                    egui::RichText::new(&date_str).monospace(),
                )
                .selectable(false),
            );

            let name_resp =
                name_resp.on_hover_cursor(CursorIcon::PointingHand);
            if name_resp.clicked() {
                *open_path = Some(entry.path.clone());
            }
            if name_resp.secondary_clicked() {
                *open_parent =
                    entry.path.parent().map(|p| p.to_path_buf());
            }
        });
    }
    if results.len() > limit {
        ui.colored_label(
            Color32::YELLOW,
            format!(
                "… and {} more (not shown to keep the UI responsive)",
                results.len() - limit
            ),
        );
    }
}
