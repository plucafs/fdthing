use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── Sorting ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum SortOrder {
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
    pub const ALL: [SortOrder; 8] = [
        SortOrder::NameAsc,
        SortOrder::NameDesc,
        SortOrder::PathAsc,
        SortOrder::PathDesc,
        SortOrder::SizeAsc,
        SortOrder::SizeDesc,
        SortOrder::ModifiedAsc,
        SortOrder::ModifiedDesc,
    ];

    pub fn label(self) -> &'static str {
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

pub fn file_name_key(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase()
}

// ── Results ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ResultEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
}

pub fn format_size(bytes: u64) -> String {
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

pub fn format_date(ts: i64) -> String {
    if ts <= 0 {
        return "—".into();
    }
    let secs = ts;
    let days_since_epoch = secs / 86400;
    let (y, m, d) = civil_from_days(days_since_epoch);
    let remaining = secs % 86400;
    let h = remaining / 3600;
    let min = (remaining % 3600) / 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
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

#[derive(Clone, Serialize, Deserialize)]
pub struct SearchDir {
    pub id: u64,
    pub path: PathBuf,
    pub enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DirGroup {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub collapsed: bool,
    pub dirs: Vec<SearchDir>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct DirTree {
    pub free: Vec<SearchDir>,
    pub groups: Vec<DirGroup>,
    pub next_id: u64,
}

impl DirTree {
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn enabled_paths(&self) -> Vec<PathBuf> {
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

    pub fn total_dirs(&self) -> usize {
        self.free.len() + self.groups.iter().map(|g| g.dirs.len()).sum::<usize>()
    }

    pub fn group_mut(&mut self, group_id: u64) -> Option<&mut DirGroup> {
        self.groups.iter_mut().find(|g| g.id == group_id)
    }

    pub fn remove_dir(&mut self, dir_id: u64) -> Option<SearchDir> {
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
pub struct PersistedConfig {
    #[serde(default)]
    pub dir_tree: DirTree,
    pub pattern: String,
    pub case_sensitive: bool,
    pub use_regex: bool,
    pub show_hidden: bool,
    pub dirs_only: bool,
    pub follow_symlinks: bool,
    pub respect_ignorefiles: bool,
    pub ext_filter: String,
    pub sort_order: SortOrder,
    #[serde(default)]
    pub exclude_filter: String,
    #[serde(default)]
    pub empty_dirs: Vec<SearchDir>,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    #[serde(default)]
    pub result_view: ResultView,
}

fn default_ui_scale() -> f32 {
    1.40
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("fdthing").join("config.json"))
}

pub fn load_config() -> Option<PersistedConfig> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    let mut cfg: PersistedConfig = serde_json::from_str(&text).ok()?;
    // Migrate legacy flat list
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

// ── App enums ─────────────────────────────────────────────────────────────

pub enum SearchStatus {
    Idle,
    Searching,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    Pattern,
    Ext,
    Exclude,
}

#[derive(PartialEq, Eq, Clone, Copy, Serialize, Deserialize, Default)]
pub enum ResultView {
    #[default]
    Aligned,
    Fluid,
    #[serde(other)]
    Simple,
}

impl ResultView {
    pub fn label(self) -> &'static str {
        match self {
            ResultView::Aligned => "Table (aligned)",
            ResultView::Fluid => "Table (fluid)",
            ResultView::Simple => "Table (compact)",
        }
    }
}
