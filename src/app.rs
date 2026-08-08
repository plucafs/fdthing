use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

use eframe::egui;

use crate::search;
use crate::types::{
    config_path, load_config, DirTree, FocusTarget, PersistedConfig, ResultEntry, ResultView,
    SearchDir, SearchStatus, SortOrder,
};

pub struct FdGuiApp {
    // ── Directories ──────────────────────────────────────────────────────
    pub dir_tree: DirTree,
    pub new_dir_path: String,
    pub new_group_name: String,

    // ── Search parameters ────────────────────────────────────────────────
    pub pattern: String,
    pub case_sensitive: bool,
    pub use_regex: bool,
    pub show_hidden: bool,
    pub dirs_only: bool,
    pub follow_symlinks: bool,
    pub respect_ignorefiles: bool,
    pub ext_filter: String,
    pub exclude_filter: String,

    // ── Results presentation ─────────────────────────────────────────────
    pub sort_order: SortOrder,
    pub needs_sort: bool,

    // ── Appearance ───────────────────────────────────────────────────────
    pub ui_scale: f32,
    pub light_mode: bool,
    pub result_view: ResultView,

    // ── Search runtime state ─────────────────────────────────────────────
    pub results: Vec<ResultEntry>,
    pub search_status: SearchStatus,
    pub cancel_flag: Option<Arc<AtomicBool>>,
    result_receiver: Option<mpsc::Receiver<ResultEntry>>,
    pub error_message: Option<String>,
    pub info_message: Option<String>,

    // ── UI state ─────────────────────────────────────────────────────────
    pub focus_after_search: Option<FocusTarget>,
    pub pending_enable: Vec<(u64, bool)>,
    pub show_about: bool,
}

impl FdGuiApp {
    pub fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut app = Self {
            dir_tree: DirTree::default(),
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
            sort_order: SortOrder::NameAsc,
            needs_sort: false,
            ui_scale: 1.40,
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

    pub fn save_config(&self) {
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

    pub fn start_search(&mut self) {
        // Cancel previous
        if let Some(ref flag) = self.cancel_flag {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        self.results.clear();
        self.error_message = None;
        self.info_message = None;
        self.needs_sort = false;

        match search::start_search(
            &self.dir_tree,
            &self.pattern,
            self.case_sensitive,
            self.use_regex,
            self.show_hidden,
            self.follow_symlinks,
            self.respect_ignorefiles,
            &self.ext_filter,
            &self.exclude_filter,
            self.dirs_only,
        ) {
            Ok((status, cancel, rx)) => {
                self.search_status = status;
                self.cancel_flag = Some(cancel);
                self.result_receiver = Some(rx);
                self.save_config();
            }
            Err(msg) => {
                self.error_message = Some(msg);
                self.search_status = SearchStatus::Idle;
            }
        }
    }

    fn collect_and_sort(&mut self) {
        self.search_status = search::collect_results(
            &mut self.result_receiver,
            &mut self.results,
            &mut self.cancel_flag,
        );
        if self.needs_sort {
            search::apply_sort(&mut self.results, self.sort_order);
            self.needs_sort = false;
        }
    }
}

// ── eframe App ────────────────────────────────────────────────────────────

impl eframe::App for FdGuiApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Sync state
        self.ui_scale = ctx.zoom_factor();
        ctx.set_visuals(if self.light_mode {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        });

        self.collect_and_sort();

        if matches!(self.search_status, SearchStatus::Searching) {
            ctx.request_repaint();
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::viewport::ViewportCommand::Close);
        }

        // Apply pending enables
        let pending = std::mem::take(&mut self.pending_enable);
        for (dir_id, on) in pending {
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

        // UI
        crate::ui::menu::show(ctx, self);
        crate::ui::top_bar::show(ctx, self);
        crate::ui::dir_tree::show(ctx, self);
        crate::ui::results::show(ctx, self);
    }
}
