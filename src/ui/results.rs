use std::path::PathBuf;

use eframe::egui;
use egui::{Color32, CursorIcon, ScrollArea, Sense};

use crate::app::FdGuiApp;
use crate::types::{format_date, format_size, ResultView, SortOrder, TypeFilter};

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
            ui.separator();
            ui.label("Filter by type:");
            egui::ComboBox::from_id_salt("type_filter_combo")
                .selected_text(app.type_filter.label())
                .show_ui(ui, |ui| {
                    for f in TypeFilter::ALL {
                        ui.selectable_value(
                            &mut app.type_filter,
                            f,
                            f.label(),
                        );
                    }
                });
            ui.separator();
            ui.label("Filter by ext:");
            let mut ext_counts: Vec<(String, usize)> = Vec::new();
            for r in &app.results {
                if r.is_dir {
                    continue;
                }
                if let Some(ext) =
                    r.path.extension().and_then(|e| e.to_str())
                {
                    let ext = ext.to_ascii_lowercase();
                    match ext_counts
                        .iter_mut()
                        .find(|(k, _)| *k == ext)
                    {
                        Some((_, n)) => *n += 1,
                        None => ext_counts.push((ext, 1)),
                    }
                }
            }
            ext_counts
                .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let selected_ext = if app.ext_view_filter.is_empty() {
                "All".to_string()
            } else {
                let count = ext_counts
                    .iter()
                    .find(|(k, _)| *k == app.ext_view_filter)
                    .map(|(_, n)| *n)
                    .unwrap_or(0);
                format!(".{} ({count})", app.ext_view_filter)
            };
            egui::ComboBox::from_id_salt("ext_filter_combo")
                .selected_text(selected_ext)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut app.ext_view_filter,
                        String::new(),
                        "All",
                    );
                    for (ext, count) in &ext_counts {
                        ui.selectable_value(
                            &mut app.ext_view_filter,
                            ext.clone(),
                            format!(".{ext} ({count})"),
                        );
                    }
                });
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
        let mut open_folder: Option<PathBuf> = None;
        let mut open_parent: Option<PathBuf> = None;
        let scroll_to_top = std::mem::take(&mut app.scroll_to_top);
        let mut selected_path = app.selected_path.clone();
        let type_filter = app.type_filter;
        let ext_filter = app.ext_view_filter.clone();

        // Results
        match app.result_view {
            ResultView::Aligned => {
                ui.set_min_width(row_w);
                let mut scroll = ScrollArea::both().auto_shrink([false; 2]);
                if scroll_to_top {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }
                scroll.show(ui, |ui| {
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
                        &mut open_folder,
                        &mut open_parent,
                        &mut selected_path,
                        type_filter,
                        &ext_filter,
                    );
                });
            }
            ResultView::Fluid => {
                let mut scroll = ScrollArea::both().auto_shrink([false; 2]);
                if scroll_to_top {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }
                scroll.show(ui, |ui| {
                    let limit = 10_000;
                    for entry in app
                        .results
                        .iter()
                        .filter(|e| matches_filters(e, type_filter, &ext_filter))
                        .take(limit)
                    {
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
                        let is_selected =
                            selected_path.as_ref() == Some(&entry.path);

                        ui.horizontal(|ui| {
                            let name = format!("{icon} {file_name}");
                            let name_text = if is_selected {
                                egui::RichText::new(name)
                                    .color(
                                        ui.visuals().selection.stroke.color,
                                    )
                                    .strong()
                            } else {
                                egui::RichText::new(name)
                            };
                            let name_resp = ui.add(
                                egui::Label::new(name_text)
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
                                    selected_path =
                                        Some(entry.path.clone());
                                    open_path =
                                        Some(entry.path.clone());
                                }
                                name_resp.context_menu(|ui| {
                                    item_context_menu(
                                        ui,
                                        entry,
                                        &mut open_path,
                                        &mut open_folder,
                                        &mut open_parent,
                                    );
                                });
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
                let mut scroll = ScrollArea::both().auto_shrink([false; 2]);
                if scroll_to_top {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }
                scroll.show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let limit = 10_000;
                    for entry in app
                        .results
                        .iter()
                        .filter(|e| matches_filters(e, type_filter, &ext_filter))
                        .take(limit)
                    {
                        let icon =
                            if entry.is_dir { "📁" } else { "📄" };
                        let file_name = entry
                            .path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("???");
                        let parent = entry
                            .path
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let is_selected =
                            selected_path.as_ref() == Some(&entry.path);
                        let name_color = if is_selected {
                            ui.visuals().selection.stroke.color
                        } else {
                            ui.visuals().text_color()
                        };
                        let galley = layout_compact_row(
                            ui,
                            icon,
                            file_name,
                            &parent,
                            name_color,
                        );
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(
                                ui.available_width(),
                                galley.size().y,
                            ),
                            Sense::click(),
                        );
                        let resp = resp
                            .on_hover_cursor(CursorIcon::PointingHand);
                        let galley = if !is_selected && resp.hovered()
                        {
                            let hover_color =
                                ui.style().interact(&resp).text_color();
                            layout_compact_row(
                                ui,
                                icon,
                                file_name,
                                &parent,
                                hover_color,
                            )
                        } else {
                            galley
                        };
                        ui.painter().galley(
                            rect.left_top(),
                            galley,
                            ui.visuals().text_color(),
                        );
                        if resp.clicked() {
                            selected_path =
                                Some(entry.path.clone());
                            open_path = Some(entry.path.clone());
                        }
                        resp.context_menu(|ui| {
                            item_context_menu(
                                ui,
                                entry,
                                &mut open_path,
                                &mut open_folder,
                                &mut open_parent,
                            );
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
        }

        app.selected_path = selected_path;

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
        if let Some(p) = open_folder {
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
        if let Some(p) = open_parent {
            match open::that(&p) {
                Ok(()) => {
                    app.info_message =
                        Some(format!("Opened parent folder {}", p.display()));
                }
                Err(e) => {
                    app.error_message =
                        Some(format!("Cannot open {}: {e}", p.display()));
                }
            }
        }
    });
}

fn matches_filters(
    entry: &crate::types::ResultEntry,
    type_filter: TypeFilter,
    ext_filter: &str,
) -> bool {
    if !matches_type_filter(entry, type_filter) {
        return false;
    }
    if ext_filter.is_empty() {
        return true;
    }
    if entry.is_dir {
        return false;
    }
    entry
        .path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(ext_filter))
        .unwrap_or(false)
}

fn matches_type_filter(
    entry: &crate::types::ResultEntry,
    filter: TypeFilter,
) -> bool {
    match filter {
        TypeFilter::All => true,
        TypeFilter::Files => !entry.is_dir,
        TypeFilter::Folders => entry.is_dir,
    }
}

fn item_context_menu(
    ui: &mut egui::Ui,
    entry: &crate::types::ResultEntry,
    open_path: &mut Option<PathBuf>,
    open_folder: &mut Option<PathBuf>,
    open_parent: &mut Option<PathBuf>,
) {
    let folder = if entry.is_dir {
        Some(entry.path.clone())
    } else {
        entry.path.parent().map(|p| p.to_path_buf())
    };
    let parent = folder
        .as_ref()
        .and_then(|f| f.parent().map(|p| p.to_path_buf()));

    if ui.button("Open").clicked() {
        *open_path = Some(entry.path.clone());
        ui.close_menu();
    }
    if ui
        .add_enabled(folder.is_some(), egui::Button::new("Open folder"))
        .clicked()
    {
        *open_folder = folder.clone();
        ui.close_menu();
    }
    if ui
        .add_enabled(parent.is_some(), egui::Button::new("Open parent folder"))
        .clicked()
    {
        *open_parent = parent.clone();
        ui.close_menu();
    }

    ui.separator();

    if ui.button("Copy path").clicked() {
        ui.ctx().copy_text(entry.path.display().to_string());
        ui.close_menu();
    }
    if ui
        .add_enabled(folder.is_some(), egui::Button::new("Copy folder"))
        .clicked()
    {
        ui.ctx()
            .copy_text(folder.unwrap().display().to_string());
        ui.close_menu();
    }
    if ui
        .add_enabled(parent.is_some(), egui::Button::new("Copy parent folder"))
        .clicked()
    {
        ui.ctx()
            .copy_text(parent.unwrap().display().to_string());
        ui.close_menu();
    }
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
    open_folder: &mut Option<PathBuf>,
    open_parent: &mut Option<PathBuf>,
    selected_path: &mut Option<PathBuf>,
    type_filter: TypeFilter,
    ext_filter: &str,
) {
    let limit = 10_000;
    for entry in results
        .iter()
        .filter(|e| matches_filters(e, type_filter, ext_filter))
        .take(limit)
    {
        let is_selected = selected_path.as_ref() == Some(&entry.path);
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
            let name_text = if is_selected {
                egui::RichText::new(name)
                    .color(ui.visuals().selection.stroke.color)
                    .strong()
            } else {
                egui::RichText::new(name)
            };
            let mut name_label = egui::Label::new(name_text)
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
                *selected_path = Some(entry.path.clone());
                *open_path = Some(entry.path.clone());
            }
            name_resp.context_menu(|ui| {
                item_context_menu(
                    ui,
                    entry,
                    open_path,
                    open_folder,
                    open_parent,
                );
            });
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

fn layout_compact_row(
    ui: &egui::Ui,
    icon: &str,
    file_name: &str,
    parent: &str,
    name_color: Color32,
) -> std::sync::Arc<egui::Galley> {
    let mut font_id = egui::FontSelection::Default.resolve(ui.style());
    font_id.size = (font_id.size * 0.85).round();
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &format!("{icon} {file_name}"),
        0.0,
        egui::text::TextFormat {
            font_id: font_id.clone(),
            color: name_color,
            ..Default::default()
        },
    );
    job.append(
        &format!(" {parent}"),
        0.0,
        egui::text::TextFormat {
            font_id,
            color: ui.visuals().weak_text_color(),
            ..Default::default()
        },
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.max_width = ui.available_width();
    ui.fonts(|f| f.layout_job(job))
}
