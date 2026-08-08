use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use eframe::egui;
use egui::{Color32, CursorIcon, ScrollArea, Sense};
use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};

// ── Sorting ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
enum SortOrder {
    None,
    NameAsc,
    NameDesc,
    PathAsc,
    PathDesc,
}

impl SortOrder {
    const ALL: [SortOrder; 5] = [
        SortOrder::None,
        SortOrder::NameAsc,
        SortOrder::NameDesc,
        SortOrder::PathAsc,
        SortOrder::PathDesc,
    ];

    fn label(self) -> &'static str {
        match self {
            SortOrder::None => "As found",
            SortOrder::NameAsc => "Name ↑",
            SortOrder::NameDesc => "Name ↓",
            SortOrder::PathAsc => "Path ↑",
            SortOrder::PathDesc => "Path ↓",
        }
    }
}

fn file_name_key(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase()
}

// ── Persistence ───────────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
struct SearchDirectory {
    path: PathBuf,
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
struct PersistedConfig {
    directories: Vec<SearchDirectory>,
    pattern: String,
    case_sensitive: bool,
    use_regex: bool,
    show_hidden: bool,
    follow_symlinks: bool,
    respect_ignorefiles: bool,
    ext_filter: String,
    sort_order: SortOrder,
    // serde(default) keeps configs written by older versions loadable
    #[serde(default = "default_ui_scale")]
    ui_scale: f32,
}

fn default_ui_scale() -> f32 {
    1.0
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("fd-gui").join("config.json"))
}

fn load_config() -> Option<PersistedConfig> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

// ── App state ─────────────────────────────────────────────────────────────

enum SearchStatus {
    Idle,
    Searching,
    Done,
}

struct FdGuiApp {
    // ── Directories ──────────────────────────────────────────────────────
    directories: Vec<SearchDirectory>,
    new_dir_path: String,

    // ── Search parameters ────────────────────────────────────────────────
    pattern: String,
    case_sensitive: bool,
    use_regex: bool,
    show_hidden: bool,
    follow_symlinks: bool,
    respect_ignorefiles: bool,
    ext_filter: String,

    // ── Results presentation ─────────────────────────────────────────────
    sort_order: SortOrder,
    needs_sort: bool,

    // ── Appearance ───────────────────────────────────────────────────────
    ui_scale: f32,

    // ── Search runtime state ─────────────────────────────────────────────
    results: Vec<(PathBuf, bool)>, // (path, is_dir)
    search_status: SearchStatus,
    cancel_flag: Option<Arc<AtomicBool>>,
    result_receiver: Option<mpsc::Receiver<(PathBuf, bool)>>,
    error_message: Option<String>,
    info_message: Option<String>,
}

impl FdGuiApp {
    fn new() -> Self {
        // Default: current working directory as the only search path
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut app = Self {
            directories: vec![SearchDirectory {
                path: cwd,
                enabled: true,
            }],
            new_dir_path: String::new(),
            pattern: String::new(),
            case_sensitive: false,
            use_regex: false,
            show_hidden: false,
            follow_symlinks: false,
            respect_ignorefiles: true,
            ext_filter: String::new(),
            sort_order: SortOrder::None,
            needs_sort: false,
            ui_scale: 1.0,
            results: Vec::new(),
            search_status: SearchStatus::Idle,
            cancel_flag: None,
            result_receiver: None,
            error_message: None,
            info_message: None,
        };

        // Restore persisted state (overrides defaults — an empty saved
        // directory list is respected, it's the user's explicit choice)
        if let Some(cfg) = load_config() {
            app.directories = cfg.directories;
            app.pattern = cfg.pattern;
            app.case_sensitive = cfg.case_sensitive;
            app.use_regex = cfg.use_regex;
            app.show_hidden = cfg.show_hidden;
            app.follow_symlinks = cfg.follow_symlinks;
            app.respect_ignorefiles = cfg.respect_ignorefiles;
            app.ext_filter = cfg.ext_filter;
            app.sort_order = cfg.sort_order;
            app.ui_scale = cfg.ui_scale;
        }
        app
    }

    fn save_config(&self) {
        let Some(path) = config_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let cfg = PersistedConfig {
            directories: self.directories.clone(),
            pattern: self.pattern.clone(),
            case_sensitive: self.case_sensitive,
            use_regex: self.use_regex,
            show_hidden: self.show_hidden,
            follow_symlinks: self.follow_symlinks,
            respect_ignorefiles: self.respect_ignorefiles,
            ext_filter: self.ext_filter.clone(),
            sort_order: self.sort_order,
            ui_scale: self.ui_scale,
        };
        if let Ok(json) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Cancel an in-flight search and launch a new one.
    fn start_search(&mut self) {
        if let Some(ref flag) = self.cancel_flag {
            flag.store(true, Ordering::Relaxed);
        }

        self.results.clear();
        self.error_message = None;
        self.info_message = None;
        self.needs_sort = false;

        let search_paths: Vec<PathBuf> = self
            .directories
            .iter()
            .filter(|d| d.enabled)
            .map(|d| d.path.clone())
            .collect();

        if search_paths.is_empty() {
            self.error_message = Some("No directories enabled for search.".into());
            self.search_status = SearchStatus::Idle;
            return;
        }

        if self.pattern.trim().is_empty() {
            self.error_message = Some("Enter a search pattern.".into());
            self.search_status = SearchStatus::Idle;
            return;
        }

        // Compile regex if the user wants regex matching
        let regex: Option<Regex> = if self.use_regex {
            let re_str = if self.case_sensitive {
                self.pattern.clone()
            } else {
                format!("(?i){}", self.pattern)
            };
            match Regex::new(&re_str) {
                Ok(re) => Some(re),
                Err(e) => {
                    self.error_message = Some(format!("Invalid regex: {e}"));
                    self.search_status = SearchStatus::Idle;
                    return;
                }
            }
        } else {
            None
        };

        // Parse extension filter: ".rs, py; txt" → ["rs", "py", "txt"]
        let exts: Vec<String> = self
            .ext_filter
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .map(|s| s.trim().trim_start_matches('.'))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect();

        let mut builder = WalkBuilder::new(&search_paths[0]);
        for p in &search_paths[1..] {
            builder.add(p);
        }
        builder.hidden(!self.show_hidden);
        builder.follow_links(self.follow_symlinks);
        builder.git_ignore(self.respect_ignorefiles);
        builder.git_global(self.respect_ignorefiles);
        builder.git_exclude(self.respect_ignorefiles);
        builder.ignore(self.respect_ignorefiles);
        builder.max_depth(None); // no limit, like fd default

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel.clone());

        let (tx, rx) = mpsc::channel();
        self.result_receiver = Some(rx);

        let pattern = self.pattern.clone();
        let pattern_lower = pattern.to_lowercase();
        let case_sensitive = self.case_sensitive;

        // Run the walker on a background thread
        thread::spawn(move || {
            for result in builder.build() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }

                if let Ok(entry) = result {
                    let path = entry.path();

                    // Extension filter (on next search, like fd -e)
                    if !exts.is_empty() {
                        let ext_ok = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| {
                                let e = e.to_lowercase();
                                exts.iter().any(|x| x == &e)
                            })
                            .unwrap_or(false);
                        if !ext_ok {
                            continue;
                        }
                    }

                    // Match against the file name (like fd default)
                    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                        let hit = if let Some(ref re) = regex {
                            re.is_match(file_name)
                        } else if case_sensitive {
                            file_name.contains(&pattern)
                        } else {
                            file_name.to_lowercase().contains(&pattern_lower)
                        };

                        if hit {
                            let is_dir =
                                entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let _ = tx.send((path.to_path_buf(), is_dir));
                        }
                    }
                }
            }
            // Sender dropped here → receiver gets Disconnected
        });

        self.search_status = SearchStatus::Searching;
        self.save_config(); // persist options + pattern on every search
    }

    /// Drain any available results from the background thread.
    fn collect_results(&mut self) {
        if let Some(ref rx) = self.result_receiver {
            loop {
                match rx.try_recv() {
                    Ok(item) => self.results.push(item),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.search_status = SearchStatus::Done;
                        self.result_receiver = None;
                        self.cancel_flag = None;
                        self.needs_sort = true; // sort once the list is complete
                        break;
                    }
                }
            }
        }
    }

    fn apply_sort(&mut self) {
        match self.sort_order {
            SortOrder::None => {} // keep insertion order
            SortOrder::NameAsc => self.results.sort_by_key(|(p, _)| file_name_key(p)),
            SortOrder::NameDesc => self
                .results
                .sort_by_key(|(p, _)| Reverse(file_name_key(p))),
            SortOrder::PathAsc => self.results.sort(),
            SortOrder::PathDesc => self.results.sort_by(|a, b| b.cmp(a)),
        }
    }
}

// ── eframe App ────────────────────────────────────────────────────────────

impl eframe::App for FdGuiApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Mirror egui's zoom factor into our persisted state (the user can
        // also change it with Ctrl + / Ctrl - / Ctrl 0 keyboard shortcuts)
        self.ui_scale = ctx.zoom_factor();

        // Drain results from the search thread every frame
        self.collect_results();
        if self.needs_sort {
            self.apply_sort();
            self.needs_sort = false;
        }

        // Keep repainting while results stream in
        if matches!(self.search_status, SearchStatus::Searching) {
            ctx.request_repaint();
        }

        // ── Top bar: search input + options ──────────────────────────────
        egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
            ui.vertical(|ui| {
                // Row 0: pattern + action buttons
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.pattern)
                            .hint_text("Search pattern (file name)…")
                            .desired_width(300.0),
                    );

                    let enter_pressed = response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    if ui.button("Search").clicked() || enter_pressed {
                        self.start_search();
                    }

                    if matches!(self.search_status, SearchStatus::Searching)
                        && ui.button("⏹ Stop").clicked()
                    {
                        if let Some(ref flag) = self.cancel_flag {
                            flag.store(true, Ordering::Relaxed);
                        }
                        self.search_status = SearchStatus::Done;
                        self.needs_sort = true;
                    }

                    if ui.button("Clear").clicked() {
                        self.pattern.clear();
                        self.results.clear();
                        self.search_status = SearchStatus::Idle;
                        self.error_message = None;
                        self.info_message = None;
                    }
                });

                // Row 1: option toggles + extension filter
                ui.horizontal(|ui| {
                    let mut opts_changed = false;
                    opts_changed |= ui
                        .checkbox(&mut self.case_sensitive, "Aa case")
                        .changed();
                    opts_changed |= ui.checkbox(&mut self.use_regex, ".* regex").changed();
                    opts_changed |= ui.checkbox(&mut self.show_hidden, "Hidden").changed();
                    opts_changed |= ui
                        .checkbox(&mut self.follow_symlinks, "Symlinks")
                        .changed();
                    opts_changed |= ui
                        .checkbox(&mut self.respect_ignorefiles, ".gitignore")
                        .changed();
                    if opts_changed {
                        self.save_config();
                    }

                    ui.separator();
                    ui.label("Ext:");
                    let ext_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.ext_filter)
                            .hint_text("rs, py, txt…")
                            .desired_width(110.0),
                    );
                    if ext_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        self.start_search();
                    }

                    ui.separator();
                    ui.label("Scale:");
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.ui_scale)
                                .speed(0.02)
                                .range(0.5..=3.0)
                                .max_decimals(2)
                                .suffix("×"),
                        )
                        .changed()
                    {
                        ctx.set_zoom_factor(self.ui_scale);
                        self.save_config();
                    }
                });
            });
        });

        // ── Left panel: directory management ─────────────────────────────
        egui::SidePanel::left("dir_panel")
            .min_width(260.0)
            .show(ctx, |ui| {
                ui.heading("📁 Search Directories");
                ui.separator();

                // Add new directory
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_dir_path)
                            .hint_text("Path…")
                            .desired_width(160.0),
                    );
                    if ui.button("📂 Browse…").clicked() {
                        // Native folder picker (blocks the UI thread but
                        // runs its own event loop on most platforms)
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.new_dir_path = path.display().to_string();
                        }
                    }
                });

                if ui.button("➕ Add").clicked() && !self.new_dir_path.trim().is_empty() {
                    let p = PathBuf::from(self.new_dir_path.trim());
                    if p.is_dir() && !self.directories.iter().any(|d| d.path == p) {
                        self.directories.push(SearchDirectory {
                            path: p,
                            enabled: true,
                        });
                        self.new_dir_path.clear();
                        self.save_config();
                    }
                }

                ui.separator();

                // List managed directories (extract values before the
                // horizontal closure to avoid double-borrowing self)
                let mut remove_idx: Option<usize> = None;
                ScrollArea::vertical()
                    .max_height(200.0)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for i in 0..self.directories.len() {
                            let path_display =
                                self.directories[i].path.display().to_string();
                            let mut enabled = self.directories[i].enabled;
                            let mut to_remove = false;

                            ui.horizontal(|ui| {
                                ui.checkbox(&mut enabled, "");
                                ui.label(&path_display);
                                if ui.button("🗑").clicked() {
                                    to_remove = true;
                                }
                            });

                            if self.directories[i].enabled != enabled {
                                self.directories[i].enabled = enabled;
                                self.save_config();
                            }
                            if to_remove {
                                remove_idx = Some(i);
                            }
                        }
                    });

                if let Some(idx) = remove_idx {
                    self.directories.remove(idx);
                    self.save_config();
                }

                // Quick toggles
                if !self.directories.is_empty() {
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("☑ All").clicked() {
                            for d in &mut self.directories {
                                d.enabled = true;
                            }
                            self.save_config();
                        }
                        if ui.button("☐ None").clicked() {
                            for d in &mut self.directories {
                                d.enabled = false;
                            }
                            self.save_config();
                        }
                    });
                }
            });

        // ── Central area: results ────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            // Status line + sort selector
            ui.horizontal(|ui| {
                match self.search_status {
                    SearchStatus::Idle => {
                        ui.label("Ready — type a pattern and press Search");
                    }
                    SearchStatus::Searching => {
                        ui.label(format!(
                            "🔎 Searching… {} matches so far",
                            self.results.len()
                        ));
                    }
                    SearchStatus::Done => {
                        ui.label(format!(
                            "✅ Done — {} matches found",
                            self.results.len()
                        ));
                    }
                }

                ui.separator();

                ui.label("Sort:");
                let mut sort_order = self.sort_order;
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
                    self.sort_order = sort_order;
                    self.needs_sort = true;
                    self.save_config();
                }
            });

            // Messages
            if let Some(ref err) = self.error_message {
                ui.colored_label(Color32::RED, err);
            }
            if let Some(ref info) = self.info_message {
                ui.colored_label(Color32::GRAY, info);
            }

            ui.separator();

            // Results list — clickable (collect clicks, act after the loop
            // to avoid mutating self while iterating over self.results)
            let mut open_path: Option<PathBuf> = None;
            let mut open_parent: Option<PathBuf> = None;

            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let display_limit = 10_000;
                    for (path, is_dir) in self.results.iter().take(display_limit) {
                        let icon = if *is_dir { "📁" } else { "📄" };
                        let text = format!("{icon} {}", path.display());
                        let response = ui
                            .add(egui::Label::new(text).sense(Sense::click()))
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .on_hover_text(
                                "Click: open • Right-click: open containing folder",
                            );

                        if response.clicked() {
                            open_path = Some(path.clone());
                        }
                        if response.secondary_clicked() {
                            open_parent = path.parent().map(|p| p.to_path_buf());
                        }
                    }
                    if self.results.len() > display_limit {
                        ui.colored_label(
                            Color32::YELLOW,
                            format!(
                                "… and {} more (not shown to keep the UI responsive)",
                                self.results.len() - display_limit
                            ),
                        );
                    }
                });

            // Handle open requests
            if let Some(p) = open_path {
                match open::that(&p) {
                    Ok(()) => {
                        self.info_message = Some(format!("Opened {}", p.display()));
                    }
                    Err(e) => {
                        self.error_message =
                            Some(format!("Cannot open {}: {e}", p.display()));
                    }
                }
            }
            if let Some(p) = open_parent {
                match open::that(&p) {
                    Ok(()) => {
                        self.info_message =
                            Some(format!("Opened folder {}", p.display()));
                    }
                    Err(e) => {
                        self.error_message =
                            Some(format!("Cannot open {}: {e}", p.display()));
                    }
                }
            }
        });
    }
}

// ── main ──────────────────────────────────────────────────────────────────

fn main() -> Result<(), eframe::Error> {
    // Load the application icon from embedded PNG
    let icon_bytes = include_bytes!("../assets/icon_128.png");
    let icon = eframe::icon_data::from_png_bytes(icon_bytes)
        .expect("embedded icon must be valid PNG");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_title("fd-gui — a graphical file finder")
            .with_icon(std::sync::Arc::new(icon)),
        ..Default::default()
    };

    eframe::run_native(
        "fd-gui",
        options,
        Box::new(|cc| {
            let app = FdGuiApp::new();
            // Apply the persisted UI scale at startup
            cc.egui_ctx.set_zoom_factor(app.ui_scale);
            Ok(Box::new(app))
        }),
    )
}
