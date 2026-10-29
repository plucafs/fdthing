use eframe::egui;
use egui::ScrollArea;

use crate::types::{normalize_exclude_token, normalize_extension_token, TokenFilter};

pub fn token_filter_ui(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    id: &'static str,
    label: &str,
    filter: &mut TokenFilter,
    add_input: &mut String,
    modal_open: &mut bool,
    extensions: bool,
) -> bool {
    let mut changed = false;

    ui.label(label);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ScrollArea::horizontal()
            .id_salt((id, "active_tokens"))
            .auto_shrink([false, true])
            .max_width(440.00)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if filter.active.is_empty() {
                        ui.weak("None");
                    }
                    for token in filter.active.clone() {
                        if ui.small_button(format!("{token} ×")).clicked() {
                            changed |= filter.set_active(&token, false);
                        }
                    }
                });
            });
    });

    let manage = ui
        .button("+")
        .on_hover_text(format!("Manage {label} filters"));
    let opened = manage.clicked();
    *modal_open |= opened;

    if *modal_open {
        let mut close = false;
        let response =
            egui::Modal::new(egui::Id::new((id, "token_filter_modal"))).show(ctx, |ui| {
                let fixed_size = egui::vec2(440.0, 360.0);
                ui.set_min_size(fixed_size);
                ui.set_max_size(fixed_size);

                ui.strong(if extensions {
                    "Extensions"
                } else {
                    "Exclude patterns"
                });
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    let input_id = ui.make_persistent_id((id, "add_input"));
                    let response = ui.add(
                        egui::TextEdit::singleline(add_input)
                            .id(input_id)
                            .hint_text(if extensions {
                                "pdf, epub…"
                            } else {
                                "target, *.log…"
                            })
                            .desired_width(330.0),
                    );
                    let enter_pressed =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let add_clicked = ui.button("Add").clicked();
                    if enter_pressed || add_clicked {
                        changed |= add_tokens(filter, add_input, extensions);
                        add_input.clear();
                        ui.memory_mut(|mem| mem.request_focus(input_id));
                    } else if opened {
                        ui.memory_mut(|mem| mem.request_focus(input_id));
                    }
                });

                ui.separator();
                if filter.known.is_empty() {
                    ui.weak("No remembered tokens");
                }

                let known = filter.known.clone();
                let mut forget = None;
                ScrollArea::vertical()
                    .id_salt((id, "known_tokens"))
                    .auto_shrink([false; 2])
                    .max_height(235.0)
                    .show(ui, |ui| {
                        for token in &known {
                            ui.horizontal(|ui| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("🗑").clicked() {
                                            forget = Some(token.clone());
                                        }
                                        let mut active = filter.active.contains(token);
                                        if ui.checkbox(&mut active, token).changed() {
                                            changed |= filter.set_active(token, active);
                                        }
                                    },
                                );
                            });
                        }
                    });

                if let Some(token) = forget {
                    changed |= filter.forget_token(&token);
                }

                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });

        if close || response.should_close() {
            *modal_open = false;
        }
    }

    changed
}

fn add_tokens(filter: &mut TokenFilter, input: &str, extensions: bool) -> bool {
    let separator = |c: char| c == ',' || c == ';' || (extensions && c.is_whitespace());
    let mut changed = false;
    for raw in input.split(separator) {
        let token = if extensions {
            normalize_extension_token(raw)
        } else {
            normalize_exclude_token(raw)
        };
        if let Some(token) = token {
            changed |= filter.add_token(token);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::add_tokens;
    use crate::types::TokenFilter;

    #[test]
    fn adding_tokens_accepts_batches_and_normalizes_extensions() {
        let mut filter = TokenFilter::default();

        assert!(add_tokens(&mut filter, ".PDF, epub;PDF", true));
        assert_eq!(filter.active, ["pdf", "epub"]);
        assert_eq!(filter.known, ["epub", "pdf"]);
    }

    #[test]
    fn adding_exclude_patterns_keeps_glob_case_and_splits_delimiters() {
        let mut filter = TokenFilter::default();

        assert!(add_tokens(&mut filter, "Build, *.log;Cache", false));
        assert_eq!(filter.active, ["Build", "*.log", "Cache"]);
    }
}
