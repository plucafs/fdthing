mod app;
mod search;
mod types;
mod ui;

use app::FdGuiApp;

fn main() -> Result<(), eframe::Error> {
    let icon_bytes = include_bytes!("../assets/icon_128.png");
    let icon = eframe::icon_data::from_png_bytes(icon_bytes)
        .expect("embedded icon must be valid PNG");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_title("fdthing — a graphical file finder")
            .with_icon(std::sync::Arc::new(icon)),
        ..Default::default()
    };

    eframe::run_native(
        "fdthing",
        options,
        Box::new(|cc| {
            let app = FdGuiApp::new();
            cc.egui_ctx.set_zoom_factor(app.ui_scale);
            cc.egui_ctx.set_visuals(if app.light_mode {
                egui::Visuals::light()
            } else {
                egui::Visuals::dark()
            });
            Ok(Box::new(app))
        }),
    )
}
