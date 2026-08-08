use std::path::PathBuf;

use eframe::egui;
use egui::{CursorIcon, ScrollArea, Sense};

use crate::app::FdGuiApp;
use crate::types::{DirGroup, SearchDir};

pub fn show(ctx: &egui::Context, app: &mut FdGuiApp) {
    egui::SidePanel::left("dir_panel")
        .min_width(280.0)
        .show(ctx, |ui| {
            ui.heading("Search Directories");
            ui.separator();

            // ── Add directory ────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.new_dir_path)
                        .hint_text("Path…")
                        .desired_width(170.0),
                );
                if ui.button("Browse").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        app.new_dir_path = path.display().to_string();
                    }
                }
            });
            if ui.button("Add directory").clicked()
                && !app.new_dir_path.trim().is_empty()
            {
                let p = PathBuf::from(app.new_dir_path.trim());
                if p.is_dir()
                    && !app.dir_tree.free.iter().any(|d| d.path == p)
                    && !app
                        .dir_tree
                        .groups
                        .iter()
                        .any(|g| g.dirs.iter().any(|d| d.path == p))
                {
                    let id = app.dir_tree.alloc_id();
                    app.dir_tree.free.push(SearchDir {
                        id,
                        path: p,
                        enabled: true,
                    });
                    app.new_dir_path.clear();
                    app.save_config();
                }
            }

            // ── New group ────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.new_group_name)
                        .hint_text("Group name…")
                        .desired_width(170.0),
                );
                if ui.button("New group").clicked()
                    && !app.new_group_name.trim().is_empty()
                {
                    let id = app.dir_tree.alloc_id();
                    app.dir_tree.groups.push(DirGroup {
                        id,
                        name: app.new_group_name.trim().to_string(),
                        enabled: true,
                        collapsed: false,
                        dirs: Vec::new(),
                    });
                    app.new_group_name.clear();
                    app.save_config();
                }
            });

            ui.separator();

            // ── Tree rendering ───────────────────────────────────────────
            let free_dirs: Vec<(usize, SearchDir)> = app
                .dir_tree
                .free
                .iter()
                .enumerate()
                .map(|(i, d)| (i, d.clone()))
                .collect();
            let groups: Vec<(usize, DirGroup)> = app
                .dir_tree
                .groups
                .iter()
                .enumerate()
                .map(|(i, g)| (i, g.clone()))
                .collect();

            let mut remove_free: Vec<usize> = Vec::new();
            let mut remove_group: Vec<usize> = Vec::new();
            let mut drop_onto_group: Option<(u64, u64)> = None;
            let mut drop_out_of_group: Option<u64> = None;
            let mut new_group_states: Vec<(usize, bool)> = Vec::new();
            let mut new_collapsed: Vec<(usize, bool)> = Vec::new();

            ScrollArea::vertical()
                .max_height(200.0)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // ── Ungroup drop target ──────────────────────────────
                    let ungroup_id = egui::Id::new("ungroup_target");
                    let ungroup_resp = ui.add(
                        egui::Label::new("Ungroup (drop here)")
                            .selectable(false),
                    );
                    let ungroup_drop = ui.interact(
                        ungroup_resp.rect,
                        ungroup_id,
                        Sense::hover(),
                    );
                    if let Some(payload) =
                        ungroup_drop.dnd_hover_payload::<u64>()
                    {
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                        ui.painter().rect_stroke(
                            ungroup_resp.rect.expand(4.0),
                            4.0,
                            egui::Stroke::new(
                                2.0,
                                ui.visuals().selection.bg_fill,
                            ),
                            egui::StrokeKind::Middle,
                        );
                        if ui.input(|i| i.pointer.any_released()) {
                            drop_out_of_group = Some(*payload);
                        }
                    }

                    // ── Free directories ─────────────────────────────────
                    for &(i, ref d) in &free_dirs {
                        let path_text = d.path.display().to_string();
                        let mut on = d.enabled;
                        let mut del = false;

                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::Label::new("::")
                                    .sense(Sense::drag())
                                    .selectable(false),
                            );
                            resp.dnd_set_drag_payload(d.id);
                            if ui.checkbox(&mut on, "").changed() {
                                app.pending_enable.push((d.id, on));
                            }
                            ui.add(
                                egui::Label::new(&path_text)
                                    .selectable(false),
                            );
                            if ui.button("🗑").clicked() {
                                del = true;
                            }
                        });

                        if del {
                            remove_free.push(i);
                        }
                    }

                    // ── Groups ───────────────────────────────────────────
                    for &(gi, ref group) in &groups {
                        let mut g_on = group.enabled;
                        let collapsed = group.collapsed;
                        let mut del_group = false;

                        let full_resp = ui
                            .horizontal(|ui| {
                                let collapse_btn = ui
                                    .add(
                                        egui::Label::new(if collapsed {
                                            "[+]"
                                        } else {
                                            "[-]"
                                        })
                                        .sense(Sense::click())
                                        .selectable(false),
                                    )
                                    .on_hover_cursor(CursorIcon::PointingHand);
                                if collapse_btn.clicked() {
                                    new_collapsed
                                        .push((gi, !group.collapsed));
                                }
                                if ui.checkbox(&mut g_on, "").changed() {
                                    new_group_states.push((gi, g_on));
                                }
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&group.name)
                                            .strong(),
                                    )
                                    .selectable(false),
                                );
                                if ui.button("🗑").clicked() {
                                    del_group = true;
                                }
                                ui.allocate_space(egui::vec2(
                                    ui.available_width(),
                                    0.0,
                                ));
                            })
                            .response;

                        if let Some(payload) =
                            full_resp.dnd_hover_payload::<u64>()
                        {
                            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                            ui.painter().rect_stroke(
                                full_resp.rect,
                                0.0,
                                egui::Stroke::new(
                                    2.0,
                                    ui.visuals().selection.bg_fill,
                                ),
                                egui::StrokeKind::Middle,
                            );
                            if ui.input(|i| i.pointer.any_released()) {
                                drop_onto_group =
                                    Some((*payload, group.id));
                            }
                        }

                        if del_group {
                            remove_group.push(gi);
                            continue;
                        }

                        // Children
                        if !collapsed {
                            let children: Vec<_> =
                                group.dirs.iter().cloned().collect();
                            let mut remove_child: Vec<usize> = Vec::new();
                            for (ci, child) in children.iter().enumerate() {
                                let path_text =
                                    child.path.display().to_string();
                                let mut on = child.enabled;
                                let mut del_child = false;
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::Label::new("   ")
                                            .selectable(false),
                                    );
                                    let child_resp = ui.add(
                                        egui::Label::new("::")
                                            .sense(Sense::drag())
                                            .selectable(false),
                                    );
                                    child_resp
                                        .dnd_set_drag_payload(child.id);
                                    if ui
                                        .checkbox(&mut on, "")
                                        .changed()
                                    {
                                        app.pending_enable
                                            .push((child.id, on));
                                    }
                                    ui.add(
                                        egui::Label::new(&path_text)
                                            .selectable(false),
                                    );
                                    if ui.button("🗑").clicked() {
                                        del_child = true;
                                    }
                                });
                                if del_child {
                                    remove_child.push(ci);
                                }
                            }
                            if !remove_child.is_empty() {
                                let g = &mut app.dir_tree.groups[gi];
                                for ci in remove_child.iter().rev() {
                                    g.dirs.remove(*ci);
                                }
                                app.save_config();
                            }
                        }
                    }

                    // ── Drop target for free area ────────────────────────
                    let free_area_id = egui::Id::new("free_drop_zone");
                    let free_resp = ui.interact(
                        ui.max_rect(),
                        free_area_id,
                        Sense::hover(),
                    );
                    if let Some(payload) =
                        free_resp.dnd_hover_payload::<u64>()
                    {
                        if ui.input(|i| i.pointer.any_released()) {
                            drop_out_of_group = Some(*payload);
                        }
                    }
                });

            // ── Apply deferred actions ───────────────────────────────────
            let pending = std::mem::take(&mut app.pending_enable);
            for &(dir_id, on) in &pending {
                if let Some(idx) =
                    app.dir_tree.free.iter().position(|d| d.id == dir_id)
                {
                    app.dir_tree.free[idx].enabled = on;
                } else {
                    for g in &mut app.dir_tree.groups {
                        if let Some(idx) =
                            g.dirs.iter().position(|d| d.id == dir_id)
                        {
                            g.dirs[idx].enabled = on;
                            break;
                        }
                    }
                }
            }

            for i in remove_free.iter().rev() {
                app.dir_tree.free.remove(*i);
            }
            for gi in remove_group.iter().rev() {
                app.dir_tree.groups.remove(*gi);
            }
            if let Some((dir_id, group_id)) = drop_onto_group {
                if let Some(d) = app.dir_tree.remove_dir(dir_id) {
                    if let Some(g) = app.dir_tree.group_mut(group_id) {
                        g.dirs.push(d);
                    }
                }
            }
            if let Some(dir_id) = drop_out_of_group {
                if drop_onto_group.is_none() {
                    if let Some(d) = app.dir_tree.remove_dir(dir_id) {
                        app.dir_tree.free.push(d);
                    }
                }
            }
            for &(gi, on) in &new_group_states {
                app.dir_tree.groups[gi].enabled = on;
            }
            for &(gi, coll) in &new_collapsed {
                app.dir_tree.groups[gi].collapsed = coll;
            }

            if !remove_free.is_empty()
                || !remove_group.is_empty()
                || drop_onto_group.is_some()
                || drop_out_of_group.is_some()
                || !new_group_states.is_empty()
                || !new_collapsed.is_empty()
            {
                app.save_config();
            }
        });
}
