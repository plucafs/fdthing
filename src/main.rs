use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use eframe::egui;
use egui::{Color32, CursorIcon, ScrollArea, Sense};
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};

// ── Sorting ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
enum SortOrder {
    NameAsc,
    NameDesc,
    PathAsc,
    PathDesc,
    SizeAsc,
    SizeDesc,
    ModifiedAsc,
    #[serde(other)]
    ModifiedDesc,
}

impl SortOrder {
    const ALL: [SortOrder; 8] = [
        SortOrder::NameAsc,
        SortOrder::NameDesc,
        SortOrder::PathAsc,
        SortOrder::PathDesc,
        SortOrder::SizeAsc,
        SortOrder::SizeDesc,
        SortOrder::ModifiedAsc,
        SortOrder::ModifiedDesc,
    ];

    fn label(self) -> &'static str {
        match self {
            SortOrder::NameAsc => "Name (Asc)",
            SortOrder::NameDesc => "Name (Des)",
            SortOrder::PathAsc => "Path (Asc)",
            SortOrder::PathDesc => "Path (Des)",
            SortOrder::SizeAsc => "Size (Asc)",
            SortOrder::SizeDesc => "Size (Des)",
            SortOrder::ModifiedAsc => "Modified (Asc)",
            SortOrder::ModifiedDesc => "Modified (Des)",
        }
    }
}

fn file_name_key(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase()
}

// ── Results ──────────────────────────────────────────────────────────────

/// One search result with metadata.
#[derive(Clone)]
struct ResultEntry {
    path: PathBuf,
    is_dir: bool,
    size: u64,
    modified: i64,  // unix timestamp seconds
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        "—".into()
    } else if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_date(ts: i64) -> String {
    if ts <= 0 {
        return "—".into();
    }
    // Use chrono or manual — keep it simple: YYYY-MM-DD HH:MM
    let secs = ts;
    let days_since_epoch = secs / 86400;
    // Gregorian approximation (good enough for display)
    let (y, m, d) = civil_from_days(days_since_epoch);
    let remaining = secs % 86400;
    let h = remaining / 3600;
    let min = (remaining % 3600) / 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}

/// Approximate Gregorian calendar from days since Unix epoch (1970-01-01).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // Based on Howard Hinnant's algorithm
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as i64, d as i64)
}

// ── Directory groups ──────────────────────────────────────────────────────

/// A search directory with a unique id for drag & drop.
#[derive(Clone, Serialize, Deserialize)]
struct SearchDir {
    id: u64,
    path: PathBuf,
    enabled: bool,
}

/// A named group that can contain directories.
#[derive(Clone, Serialize, Deserialize)]
struct DirGroup {
    id: u64,
    name: String,
    enabled: bool,
    collapsed: bool,
    dirs: Vec<SearchDir>,
}

/// Flat list of free directories + groups.
#[derive(Clone, Serialize, Deserialize, Default)]
struct DirTree {
    free: Vec<SearchDir>,
    groups: Vec<DirGroup>,
    next_id: u64,
}

impl DirTree {
    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Collect every enabled directory path.
    fn enabled_paths(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for d in &self.free {
            if d.enabled {
                out.push(d.path.clone());
            }
        }
        for g in &self.groups {
            if g.enabled {
                for d in &g.dirs {
                    if d.enabled {
                        out.push(d.path.clone());
                    }
                }
            }
        }
        out
    }

    /// Total directory count.
    fn total_dirs(&self) -> usize {
        self.free.len() + self.groups.iter().map(|g| g.dirs.len()).sum::<usize>()
    }

    /// Find mutable ref to group.
    fn group_mut(&mut self, group_id: u64) -> Option<&mut DirGroup> {
        self.groups.iter_mut().find(|g| g.id == group_id)
    }

    /// Remove a directory from its current location.
    fn remove_dir(&mut self, dir_id: u64) -> Option<SearchDir> {
        if let Some(idx) = self.free.iter().position(|d| d.id == dir_id) {
            Some(self.free.remove(idx))
        } else {
            for g in &mut self.groups {
                if let Some(idx) = g.dirs.iter().position(|d| d.id == dir_id) {
                    return Some(g.dirs.remove(idx));
                }
            }
            None
        }
    }
}

// ── Persistence ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct PersistedConfig {
    #[serde(default)]
    dir_tree: DirTree,
    pattern: String,
    case_sensitive: bool,
    use_regex: bool,
    show_hidden: bool,
    dirs_only: bool,
    follow_symlinks: bool,
    respect_ignorefiles: bool,
    ext_filter: String,
    sort_order: SortOrder,
    #[serde(default)]
    exclude_filter: String,
    #[serde(default)]
    empty_dirs: Vec<SearchDir>, // migrate legacy flat list on load
    #[serde(default = "default_ui_scale")]
    ui_scale: f32,
    #[serde(default)]
    result_view: ResultView,
}

fn default_ui_scale() -> f32 {
    1.0
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("fd-gui").join("config.json"))
}

fn load_config() -> Option<PersistedConfig> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    let mut cfg: PersistedConfig = serde_json::from_str(&text).ok()?;
    // Migrate legacy flat list (use mem::take to avoid partial move)
    let empty_dirs = std::mem::take(&mut cfg.empty_dirs);
    if !empty_dirs.is_empty() && cfg.dir_tree.free.is_empty() {
        for (i, d) in empty_dirs.into_iter().enumerate() {
            cfg.dir_tree.free.push(SearchDir {
                id: i as u64,
                path: d.path,
                enabled: d.enabled,
            });
        }
        cfg.dir_tree.next_id = cfg.dir_tree.free.len() as u64;
    }
    if cfg.dir_tree.next_id == 0 {
        cfg.dir_tree.next_id = cfg.dir_tree.total_dirs() as u64 + 10;
    }
    // Ensure every directory has an id (repair broken data)
    let mut max_id = cfg.dir_tree.next_id;
    for d in &cfg.dir_tree.free {
        max_id = max_id.max(d.id);
    }
    for g in &cfg.dir_tree.groups {
        for d in &g.dirs {
            max_id = max_id.max(d.id);
        }
    }
    cfg.dir_tree.next_id = max_id + 1;
    Some(cfg)
}

// ── App state ─────────────────────────────────────────────────────────────

enum SearchStatus {
    Idle,
    Searching,
    Done,
}

struct FdGuiApp {
    // ── Directories ──────────────────────────────────────────────────────
    dir_tree: DirTree,
    new_dir_path: String,
    new_group_name: String,

    // ── Search parameters ────────────────────────────────────────────────
    pattern: String,
    case_sensitive: bool,
    use_regex: bool,
    show_hidden: bool,
    dirs_only: bool,
    follow_symlinks: bool,
    respect_ignorefiles: bool,
    ext_filter: String,
    exclude_filter: String,

    // ── Results presentation ─────────────────────────────────────────────
    sort_order: SortOrder,
    needs_sort: bool,

    // ── Appearance ───────────────────────────────────────────────────────
    ui_scale: f32,
    light_mode: bool,
    result_view: ResultView,

    // ── Search runtime state ─────────────────────────────────────────────
    results: Vec<ResultEntry>,
    search_status: SearchStatus,
    cancel_flag: Option<Arc<AtomicBool>>,
    result_receiver: Option<mpsc::Receiver<ResultEntry>>,
    error_message: Option<String>,
    info_message: Option<String>,

    // ── UI state ─────────────────────────────────────────────────────────
    focus_after_search: Option<FocusTarget>,
    /// (dir_id, new_enabled) to apply after the iteration (avoid borrow issues)
    pending_enable: Vec<(u64, bool)>,
    show_about: bool,
    /// Keyboard navigation in results
    selected_idx: Option<usize>,
    focus_results: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    Pattern,
    Ext,
    Exclude,
}

#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize, Default)]
enum ResultView {
    #[default]
    Aligned,
    Fluid,
    #[serde(other)]
    Simple,
}

impl ResultView {
    fn label(self) -> &'static str {
        match self {
            ResultView::Aligned => "Table (aligned)",
            ResultView::Fluid => "Table (fluid)",
            ResultView::Simple => "Table (compact)",
        }
    }
}

impl FdGuiApp {
    fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut app = Self {
            dir_tree: DirTree {
                free: Vec::new(),
                groups: Vec::new(),
                next_id: 0,
            },
            new_dir_path: String::new(),
            new_group_name: String::new(),
            pattern: String::new(),
            case_sensitive: false,
            use_regex: false,
            show_hidden: false,
            dirs_only: false,
            follow_symlinks: false,
            respect_ignorefiles: true,
            ext_filter: String::new(),
            exclude_filter: String::new(),
            sort_order: SortOrder::PathAsc,
            needs_sort: false,
            ui_scale: 1.0,
            light_mode: false,
            result_view: ResultView::Aligned,
            results: Vec::new(),
            search_status: SearchStatus::Idle,
            cancel_flag: None,
            result_receiver: None,
            error_message: None,
            info_message: None,
            focus_after_search: Some(FocusTarget::Pattern),
            pending_enable: Vec::new(),
            show_about: false,
            selected_idx: None,
            focus_results: false,
        };

        if let Some(cfg) = load_config() {
            app.dir_tree = cfg.dir_tree;
            app.pattern = cfg.pattern;
            app.case_sensitive = cfg.case_sensitive;
            app.use_regex = cfg.use_regex;
            app.show_hidden = cfg.show_hidden;
            app.dirs_only = cfg.dirs_only;
            app.follow_symlinks = cfg.follow_symlinks;
            app.respect_ignorefiles = cfg.respect_ignorefiles;
            app.ext_filter = cfg.ext_filter;
            app.exclude_filter = cfg.exclude_filter;
            app.sort_order = cfg.sort_order;
            app.ui_scale = cfg.ui_scale;
            app.result_view = cfg.result_view;
        }

        // If nothing was loaded or migrated, add CWD as default
        if app.dir_tree.total_dirs() == 0 {
            let id = app.dir_tree.alloc_id();
            app.dir_tree.free.push(SearchDir {
                id,
                path: cwd,
                enabled: true,
            });
        }

        app
    }

    fn save_config(&self) {
        let Some(path) = config_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let cfg = PersistedConfig {
            dir_tree: self.dir_tree.clone(),
            pattern: self.pattern.clone(),
            case_sensitive: self.case_sensitive,
            use_regex: self.use_regex,
            show_hidden: self.show_hidden,
            dirs_only: self.dirs_only,
            follow_symlinks: self.follow_symlinks,
            respect_ignorefiles: self.respect_ignorefiles,
            ext_filter: self.ext_filter.clone(),
            exclude_filter: self.exclude_filter.clone(),
            sort_order: self.sort_order,
            ui_scale: self.ui_scale,
            result_view: self.result_view,
            empty_dirs: Vec::new(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(path, json);
        }
    }

    fn start_search(&mut self) {
        if let Some(ref flag) = self.cancel_flag {
            flag.store(true, Ordering::Relaxed);
        }

        self.results.clear();
        self.error_message = None;
        self.info_message = None;
        self.needs_sort = false;

        let search_paths = self.dir_tree.enabled_paths();

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
        builder.max_depth(None);

        let exclude_globs: Vec<&str> = self
            .exclude_filter
            .split(|c: char| c == ',' || c == ';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !exclude_globs.is_empty() {
            let mut ov = OverrideBuilder::new(".");
            for glob in &exclude_globs {
                let _ = ov.add(&format!("!{}", glob));
            }
            if let Ok(ov) = ov.build() {
                builder.overrides(ov);
            }
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel.clone());

        let (tx, rx) = mpsc::channel();
        self.result_receiver = Some(rx);

        let pattern = self.pattern.clone();
        let pattern_lower = pattern.to_lowercase();
        let case_sensitive = self.case_sensitive;
        let dirs_only = self.dirs_only;

        thread::spawn(move || {
            for result in builder.build() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if let Ok(entry) = result {
                    let path = entry.path();
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
                            if dirs_only && !is_dir {
                                continue;
                            }
                            // Grab metadata (the walker already has it cached)
                            let meta = entry.metadata().ok();
                            let entry = ResultEntry {
                                path: path.to_path_buf(),
                                is_dir,
                                size: meta.as_ref().map_or(0, |m| m.len()),
                                modified: meta
                                    .as_ref()
                                    .and_then(|m| m.modified().ok())
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map_or(0, |d| d.as_secs() as i64),
                            };
                            let _ = tx.send(entry);
                        }
                    }
                }
            }
        });

        self.search_status = SearchStatus::Searching;
        self.save_config();
    }

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
                        self.needs_sort = true;
                        break;
                    }
                }
            }
        }
    }

    fn apply_sort(&mut self) {
        match self.sort_order {
            SortOrder::NameAsc => {
                self.results.sort_by_key(|e| file_name_key(&e.path))
            }
            SortOrder::NameDesc => self
                .results
                .sort_by_key(|e| Reverse(file_name_key(&e.path))),
            SortOrder::PathAsc => {
                self.results.sort_by(|a, b| a.path.cmp(&b.path))
            }
            SortOrder::PathDesc => {
                self.results.sort_by(|a, b| b.path.cmp(&a.path))
            }
            SortOrder::SizeAsc => self.results.sort_by_key(|e| e.size),
            SortOrder::SizeDesc => {
                self.results.sort_by_key(|e| Reverse(e.size))
            }
            SortOrder::ModifiedAsc => {
                self.results.sort_by_key(|e| e.modified)
            }
            SortOrder::ModifiedDesc => {
                self.results.sort_by_key(|e| Reverse(e.modified))
            }
        }
    }
}

// ── eframe App ────────────────────────────────────────────────────────────

impl eframe::App for FdGuiApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui_scale = ctx.zoom_factor();
        ctx.set_visuals(if self.light_mode {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        });

        self.collect_results();
        if self.needs_sort {
            self.apply_sort();
            self.needs_sort = false;
        }
        if matches!(self.search_status, SearchStatus::Searching) {
            ctx.request_repaint();
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::viewport::ViewportCommand::Close);
        }

        // ── Menu bar ─────────────────────────────────────────────────────
        egui::TopBottomPanel::top("menu_bar")
            .min_height(0.0)
            .show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(
                            egui::viewport::ViewportCommand::Close,
                        );
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });

        // ── About dialog ──────────────────────────────────────────────────
        if self.show_about {
            egui::Window::new("About fd-gui")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.heading("fd-gui");
                    ui.label("A graphical file finder for fd.");
                    ui.label(format!(
                        "Version {}",
                        env!("CARGO_PKG_VERSION")
                    ));
                    ui.separator();
                    if ui.button("Close").clicked() {
                        self.show_about = false;
                    }
                });
        }

        // Apply any deferred enable toggles from the dir tree UI
        let pending = std::mem::take(&mut self.pending_enable);
        for (dir_id, on) in pending {
            if let Some(idx) = self.dir_tree.free.iter().position(|d| d.id == dir_id) {
                self.dir_tree.free[idx].enabled = on;
            } else {
                for g in &mut self.dir_tree.groups {
                    if let Some(idx) = g.dirs.iter().position(|d| d.id == dir_id) {
                        g.dirs[idx].enabled = on;
                        break;
                    }
                }
            }
            self.save_config();
        }

        // ── Top bar ──────────────────────────────────────────────────────
        egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
            let pattern_id = egui::Id::new("pattern_input");
            let ext_id = egui::Id::new("ext_textedit");
            let excl_id = egui::Id::new("exclude_input");

            ui.vertical(|ui| {
                // Row 0
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.pattern)
                            .id(pattern_id)
                            .hint_text("Search pattern (file name)…")
                            .desired_width(300.0),
                    );
                    let enter_pressed = response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Search").clicked() || enter_pressed {
                        self.focus_after_search = Some(FocusTarget::Pattern);
                        self.start_search();
                    }
                    if matches!(self.search_status, SearchStatus::Searching)
                        && ui.button("Stop").clicked()
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

                // Row 1 — toggles + scale + theme
                ui.horizontal(|ui| {
                    let mut opts_changed = false;
                    opts_changed |= ui
                        .checkbox(&mut self.case_sensitive, "Aa case")
                        .changed();
                    opts_changed |= ui.checkbox(&mut self.use_regex, ".* regex").changed();
                    opts_changed |= ui.checkbox(&mut self.show_hidden, "Hidden").changed();
                    opts_changed |= ui.checkbox(&mut self.dirs_only, "Dirs").changed();
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
                    ui.label("UI Scale:");
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
                    ui.separator();
                    let theme_label = if self.light_mode { "Light" } else { "Dark" };
                    egui::ComboBox::from_id_salt("theme_combo")
                        .selected_text(format!("Theme: {theme_label}"))
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut self.light_mode, false, "Dark")
                                .clicked()
                                || ui
                                    .selectable_value(
                                        &mut self.light_mode,
                                        true,
                                        "Light",
                                    )
                                    .clicked()
                            {
                                self.save_config();
                            }
                        });

                    ui.separator();
                    let view_label = self.result_view.label();
                    egui::ComboBox::from_id_salt("view_combo")
                        .selected_text(format!("View: {view_label}"))
                        .show_ui(ui, |ui| {
                            let mut changed = false;
                            changed |= ui
                                .selectable_value(
                                    &mut self.result_view,
                                    ResultView::Aligned,
                                    ResultView::Aligned.label(),
                                )
                                .clicked();
                            changed |= ui
                                .selectable_value(
                                    &mut self.result_view,
                                    ResultView::Fluid,
                                    ResultView::Fluid.label(),
                                )
                                .clicked();
                            changed |= ui
                                .selectable_value(
                                    &mut self.result_view,
                                    ResultView::Simple,
                                    ResultView::Simple.label(),
                                )
                                .clicked();
                            if changed {
                                self.save_config();
                            }
                        });
                });

                // Row 2 — Ext + Exclude
                ui.horizontal(|ui| {
                    ui.label("Ext:");
                    let ext_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.ext_filter)
                            .id(ext_id)
                            .hint_text("rs, py, txt…"),
                    );
                    // Autocomplete popup
                    let popup_id = egui::Id::new("ext_popup");
                    let ext_has_focus = ui.memory_mut(|mem| mem.has_focus(ext_id));
                    if ext_has_focus && !self.ext_filter.is_empty() {
                        let token = self
                            .ext_filter
                            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                            .last()
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        if !token.is_empty() {
                            let mut suggestions: Vec<String> = self
                                .results
                                .iter()
                                .filter_map(|e| e.path.extension().and_then(|x| x.to_str()))
                                .map(|e| e.to_lowercase())
                                .filter(|e| e.starts_with(&token))
                                .collect();
                            suggestions.sort();
                            suggestions.dedup();
                            // Open or keep open the popup
                            ui.memory_mut(|mem| mem.open_popup(popup_id));
                            egui::popup_below_widget(
                                ui,
                                popup_id,
                                &ext_resp,
                                egui::PopupCloseBehavior::CloseOnClickOutside,
                                |ui| {
                                    if suggestions.is_empty() {
                                        ui.label("No extensions found (run a search first)");
                                    } else {
                                        ScrollArea::vertical()
                                            .max_height(150.0)
                                            .show(ui, |ui| {
                                                for s in suggestions.iter().take(8) {
                                                    if ui.button(s.as_str()).clicked() {
                                                        let prefix: String = self
                                                            .ext_filter
                                                            .trim_end_matches(&token)
                                                            .to_string();
                                                        self.ext_filter = if prefix.is_empty() {
                                                            s.clone()
                                                        } else {
                                                            format!("{prefix}{s}")
                                                        };
                                                        ui.memory_mut(|mem| {
                                                            mem.request_focus(ext_id);
                                                            mem.close_popup();
                                                        });
                                                    }
                                                }
                                            });
                                    }
                                },
                            );
                        }
                    } else {
                        // Close popup when ext loses focus
                        ui.memory_mut(|mem| mem.close_popup());
                    }
                    if ext_resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        self.focus_after_search = Some(FocusTarget::Ext);
                        self.start_search();
                    }
                    ui.label("Exclude:");
                    let excl_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.exclude_filter)
                            .id(excl_id)
                            .hint_text("target, *.log…"),
                    );
                    if excl_resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        self.focus_after_search = Some(FocusTarget::Exclude);
                        self.start_search();
                    }
                });

                // Deferred focus
                if let Some(ref target) = self.focus_after_search {
                    let id = match target {
                        FocusTarget::Pattern => pattern_id,
                        FocusTarget::Ext => ext_id,
                        FocusTarget::Exclude => excl_id,
                    };
                    ui.memory_mut(|mem| mem.request_focus(id));
                    self.focus_after_search = None;
                }
            });
        });

        // ── Left panel: directory tree ────────────────────────────────────
        egui::SidePanel::left("dir_panel")
            .min_width(280.0)
            .show(ctx, |ui| {
                ui.heading("Search Directories");
                ui.separator();

                // ── Add directory row ─────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_dir_path)
                            .hint_text("Path…")
                            .desired_width(170.0),
                    );
                    if ui.button("Browse").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.new_dir_path = path.display().to_string();
                        }
                    }
                });
                if ui.button("Add directory").clicked()
                    && !self.new_dir_path.trim().is_empty()
                {
                    let p = PathBuf::from(self.new_dir_path.trim());
                    if p.is_dir()
                        && !self.dir_tree.free.iter().any(|d| d.path == p)
                        && !self
                            .dir_tree
                            .groups
                            .iter()
                            .any(|g| g.dirs.iter().any(|d| d.path == p))
                    {
                        let id = self.dir_tree.alloc_id();
                        self.dir_tree.free.push(SearchDir {
                            id,
                            path: p,
                            enabled: true,
                        });
                        self.new_dir_path.clear();
                        self.save_config();
                    }
                }

                // ── New group row ─────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_group_name)
                            .hint_text("Group name…")
                            .desired_width(170.0),
                    );
                    if ui.button("New group").clicked()
                        && !self.new_group_name.trim().is_empty()
                    {
                        let id = self.dir_tree.alloc_id();
                        self.dir_tree.groups.push(DirGroup {
                            id,
                            name: self.new_group_name.trim().to_string(),
                            enabled: true,
                            collapsed: false,
                            dirs: Vec::new(),
                        });
                        self.new_group_name.clear();
                        self.save_config();
                    }
                });

                ui.separator();

                // ── Tree rendering + deferred actions ────────────────────
                // Snapshot data to avoid borrow conflicts.
                let free_dirs: Vec<(usize, SearchDir)> = self
                    .dir_tree
                    .free
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (i, d.clone()))
                    .collect();
                let groups: Vec<(usize, DirGroup)> = self
                    .dir_tree
                    .groups
                    .iter()
                    .enumerate()
                    .map(|(i, g)| (i, g.clone()))
                    .collect();

                let mut remove_free: Vec<usize> = Vec::new();
                let mut remove_group: Vec<usize> = Vec::new();
                let mut drop_onto_group: Option<(u64, u64)> = None; // (dir_id, group_id)
                let mut drop_out_of_group: Option<u64> = None; // dir_id to move to free
                let mut new_group_states: Vec<(usize, bool)> = Vec::new(); // (gi, enabled)
                let mut new_collapsed: Vec<(usize, bool)> = Vec::new();

                ScrollArea::vertical()
                    .max_height(200.0)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        // ── Ungroup drop target (pinned at top) ─────────
                        let ungroup_id = egui::Id::new("ungroup_target");
                        let ungroup_resp = ui.add(
                            egui::Label::new("Ungroup (drop here)")
                                .selectable(false),
                        );
                        // Make ungroup label a drop target (allocate a
                        // full-width interact area)
                        let ungroup_drop = ui.interact(
                            ungroup_resp.rect,
                            ungroup_id,
                            Sense::hover(),
                        );
                        if let Some(payload) =
                            ungroup_drop.dnd_hover_payload::<u64>()
                        {
                            ui.ctx()
                                .set_cursor_icon(CursorIcon::Grabbing);
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

                        // --- Free directories ---
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
                                    self.pending_enable.push((d.id, on));
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

                        // --- Groups ---
                        for &(gi, ref group) in &groups {
                            let mut g_on = group.enabled;
                            let collapsed = group.collapsed;
                            let mut del_group = false;

                            // Full-width horizontal for drop target
                            let full_resp = ui
                                .horizontal(|ui| {
                                    let collapse_btn = ui.add(
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
                                    if ui
                                        .checkbox(&mut g_on, "")
                                        .changed()
                                    {
                                        new_group_states
                                            .push((gi, g_on));
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
                                    // Eat remaining space so the response
                                    // rect spans full width
                                    ui.allocate_space(egui::vec2(
                                        ui.available_width(),
                                        0.0,
                                    ));
                                })
                                .response;

                            // Accept drops on the full group header row
                            if let Some(payload) =
                                full_resp.dnd_hover_payload::<u64>()
                            {
                                ui.ctx()
                                    .set_cursor_icon(CursorIcon::Grabbing);
                                // Visual feedback
                                ui.painter().rect_stroke(
                                    full_resp.rect,
                                    0.0,
                                    egui::Stroke::new(
                                        2.0,
                                        ui.visuals().selection.bg_fill,
                                    ),
                                    egui::StrokeKind::Middle,
                                );
                                if ui.input(|i| i.pointer.any_released())
                                {
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
                                        child_resp.dnd_set_drag_payload(
                                            child.id,
                                        );
                                        if ui
                                            .checkbox(&mut on, "")
                                            .changed()
                                        {
                                            self.pending_enable
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
                                // Remove children from this group
                                if !remove_child.is_empty() {
                                    let g = &mut self.dir_tree.groups[gi];
                                    for ci in remove_child.iter().rev() {
                                        g.dirs.remove(*ci);
                                    }
                                    self.save_config();
                                }
                            }
                        }

                        // — Drop target for free-directory area —
                        // If a directory is released here (not over any
                        // group), move it out of its group into free.
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

                // Apply deferred actions
                for &(dir_id, on) in &self.pending_enable {
                    if let Some(idx) =
                        self.dir_tree.free.iter().position(|d| d.id == dir_id)
                    {
                        self.dir_tree.free[idx].enabled = on;
                    } else {
                        for g in &mut self.dir_tree.groups {
                            if let Some(idx) =
                                g.dirs.iter().position(|d| d.id == dir_id)
                            {
                                g.dirs[idx].enabled = on;
                                break;
                            }
                        }
                    }
                }
                self.pending_enable.clear();

                for i in remove_free.iter().rev() {
                    self.dir_tree.free.remove(*i);
                }
                for gi in remove_group.iter().rev() {
                    self.dir_tree.groups.remove(*gi);
                }
                if let Some((dir_id, group_id)) = drop_onto_group {
                    if let Some(d) = self.dir_tree.remove_dir(dir_id) {
                        if let Some(g) = self.dir_tree.group_mut(group_id) {
                            g.dirs.push(d);
                        }
                    }
                }
                // Drop outside any group → move to free
                if let Some(dir_id) = drop_out_of_group {
                    // Only move if it wasn't already consumed by a group drop
                    if drop_onto_group.is_none() {
                        if let Some(d) = self.dir_tree.remove_dir(dir_id) {
                            self.dir_tree.free.push(d);
                        }
                    }
                }
                for &(gi, on) in &new_group_states {
                    self.dir_tree.groups[gi].enabled = on;
                }
                for &(gi, coll) in &new_collapsed {
                    self.dir_tree.groups[gi].collapsed = coll;
                }

                if !remove_free.is_empty()
                    || !remove_group.is_empty()
                    || drop_onto_group.is_some()
                    || drop_out_of_group.is_some()
                    || !new_group_states.is_empty()
                    || !new_collapsed.is_empty()
                {
                    self.save_config();
                }
            });

        // ── Central area ──────────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                match self.search_status {
                    SearchStatus::Idle => {
                        ui.label("Ready — type a pattern and press Search");
                    }
                    SearchStatus::Searching => {
                        ui.label(format!(
                            "Searching… {} matches so far",
                            self.results.len()
                        ));
                    }
                    SearchStatus::Done => {
                        ui.label(format!(
                            "Done — {} matches found",
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
                                .selectable_value(
                                    &mut sort_order,
                                    order,
                                    order.label(),
                                )
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

            if let Some(ref err) = self.error_message {
                egui::CollapsingHeader::new(
                    egui::RichText::new("Error").color(Color32::RED),
                )
                .default_open(true)
                .show(ui, |ui| {
                    ui.colored_label(Color32::RED, err);
                });
            }
            if let Some(ref info) = self.info_message {
                egui::CollapsingHeader::new(
                    egui::RichText::new("Info").color(Color32::GRAY),
                )
                .default_open(true)
                .show(ui, |ui| {
                    ui.colored_label(Color32::GRAY, info);
                });
            }

            ui.separator();

            // ── Column widths (fixed, not relative) ───────────────────
            let name_w = 260.0;
            let path_w = 400.0;
            let size_w = 80.0;
            let date_w = 140.0;
            let separator_w = 15.0; // ≈ width of `ui.separator()`
            let row_w = name_w + path_w + size_w + date_w + separator_w * 3.0 + 20.0;

            // ── Column headers (only for table views) ─────────────────
            if self.result_view != ResultView::Simple {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(name_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Name").selectable(false));
                // ui.separator();
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(path_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Path").selectable(false));
                // ui.separator();
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(size_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Size").selectable(false));
                // ui.separator();
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(date_w, 0.0),
                    Sense::hover(),
                );
                ui.put(r, egui::Label::new("Modified").selectable(false));
            });
            ui.separator();
            } // end column headers (only Aligned/Fluid)

            let mut open_path: Option<PathBuf> = None;
            let mut open_parent: Option<PathBuf> = None;

            // Enforce content width so horizontal scroll works
            if self.result_view == ResultView::Aligned {
            ui.set_min_width(row_w);

            ScrollArea::both()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.set_min_width(row_w);
                    let display_limit = 10_000;
                    for entry in self.results.iter().take(display_limit) {
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
                            // Fixed-width columns via allocate_exact_size.
                            // The label is placed inside the rect with ui.put,
                            // which aligns to the top-left by default.
                            let (name_rect, _) = ui.allocate_exact_size(
                                egui::vec2(name_w, 0.0),
                                Sense::hover(),
                            );
                            let name_resp = ui.put(
                                name_rect,
                                egui::Label::new(name)
                                    .truncate()
                                    .sense(Sense::click())
                                    .selectable(false),
                            );
                            ui.separator();
                            let (path_rect, _) = ui.allocate_exact_size(
                                egui::vec2(path_w, 0.0),
                                Sense::hover(),
                            );
                            ui.put(
                                path_rect,
                                egui::Label::new(parent)
                                    .truncate()
                                    .selectable(false),
                            );
                            ui.separator();
                            let (size_rect, _) = ui.allocate_exact_size(
                                egui::vec2(size_w, 0.0),
                                Sense::hover(),
                            );
                            ui.put(
                                size_rect,
                                egui::Label::new(
                                    egui::RichText::new(&size_str)
                                        .monospace(),
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
                                    egui::RichText::new(&date_str)
                                        .monospace(),
                                )
                                .selectable(false),
                            );

                            let name_resp = name_resp
                                .on_hover_cursor(CursorIcon::PointingHand);
                            if name_resp.clicked()
                            {
                                open_path = Some(entry.path.clone());
                            }
                            if name_resp.secondary_clicked() {
                                open_parent =
                                    entry.path.parent().map(|p| p.to_path_buf());
                            }
                        });
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
            } else if self.result_view == ResultView::Fluid {
                // Fluid view: columns flow naturally, no fixed widths
                ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let display_limit = 10_000;
                        for entry in self.results.iter().take(display_limit) {
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
            } else {
                // Simple view: single-line icon + full path
                ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let display_limit = 10_000;
                        for entry in self.results.iter().take(display_limit) {
                            let icon = if entry.is_dir { "📁" } else { "📄" };
                            let text = format!("{icon} {}", entry.path.display());
                            let response = ui
                                .add(
                                    egui::Label::new(text)
                                        .truncate()
                                        .sense(Sense::click()),
                                )
                                .on_hover_cursor(CursorIcon::PointingHand);
                            if response.clicked() {
                                open_path = Some(entry.path.clone());
                            }
                            if response.secondary_clicked() {
                                open_parent =
                                    entry.path.parent().map(|p| p.to_path_buf());
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
            }

            if let Some(p) = open_path {
                match open::that(&p) {
                    Ok(()) => {
                        self.info_message =
                            Some(format!("Opened {}", p.display()));
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
