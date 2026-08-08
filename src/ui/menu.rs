use eframe::egui;

use crate::app::FdGuiApp;

pub fn show(ctx: &egui::Context, app: &mut FdGuiApp) {
    egui::TopBottomPanel::top("menu_bar")
        .min_height(0.0)
        .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Restart").clicked() {
                        let current_exe =
                            std::env::current_exe().unwrap_or_default();
                        let _ = std::process::Command::new(current_exe)
                            .spawn();
                        ctx.send_viewport_cmd(
                            egui::viewport::ViewportCommand::Close,
                        );
                        ui.close_menu();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(
                            egui::viewport::ViewportCommand::Close,
                        );
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        app.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });

    // About dialog
    if app.show_about {
        egui::Window::new("About fdthing")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.heading("fdthing");
                ui.label("A graphical file finder for fd.");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.separator();
                if ui.button("Close").clicked() {
                    app.show_about = false;
                }
            });
    }
}
