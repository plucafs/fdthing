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

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TokenFilter {
    pub active: Vec<String>,
    pub known: Vec<String>,
}

impl TokenFilter {
    pub fn add_token(&mut self, token: String) -> bool {
        let mut changed = false;
        if !self.known.contains(&token) {
            self.known.push(token.clone());
            self.known.sort();
            changed = true;
        }
        if !self.active.contains(&token) {
            self.active.push(token);
            changed = true;
        }
        changed
    }

    pub fn set_active(&mut self, token: &str, active: bool) -> bool {
        if active {
            if self.active.iter().any(|known| known == token) {
                false
            } else {
                self.active.push(token.to_string());
                true
            }
        } else if let Some(index) = self.active.iter().position(|item| item == token) {
            self.active.remove(index);
            true
        } else {
            false
        }
    }

    pub fn forget_token(&mut self, token: &str) -> bool {
        let old_known_len = self.known.len();
        let old_active_len = self.active.len();
        self.known.retain(|item| item != token);
        self.active.retain(|item| item != token);
        old_known_len != self.known.len() || old_active_len != self.active.len()
    }
}

pub fn normalize_extension_token(token: &str) -> Option<String> {
    let token = token.trim().trim_start_matches('.').to_ascii_lowercase();
    (!token.is_empty()).then_some(token)
}

pub fn normalize_exclude_token(token: &str) -> Option<String> {
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

// ── Persistence ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
#[serde(default)]
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
    #[serde(default)]
    pub ext_tokens: TokenFilter,
    #[serde(default)]
    pub exclude_tokens: TokenFilter,
    // Legacy serialized filter strings retained for config migration.
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
    #[serde(default)]
    pub saved_searches: Vec<String>,
}

impl Default for PersistedConfig {
    fn default() -> Self {
        Self {
            dir_tree: DirTree::default(),
            pattern: String::new(),
            case_sensitive: false,
            use_regex: false,
            show_hidden: false,
            dirs_only: false,
            follow_symlinks: false,
            respect_ignorefiles: true,
            ext_tokens: TokenFilter::default(),
            exclude_tokens: TokenFilter::default(),
            ext_filter: String::new(),
            sort_order: SortOrder::NameAsc,
            exclude_filter: String::new(),
            empty_dirs: Vec::new(),
            ui_scale: default_ui_scale(),
            result_view: ResultView::default(),
            saved_searches: Vec::new(),
        }
    }
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
    migrate_legacy_filters(&mut cfg);
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

fn migrate_legacy_filters(cfg: &mut PersistedConfig) {
    if cfg.ext_tokens.active.is_empty() && cfg.ext_tokens.known.is_empty() {
        for token in cfg
            .ext_filter
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        {
            if let Some(token) = normalize_extension_token(token) {
                cfg.ext_tokens.add_token(token);
            }
        }
    }
    if cfg.exclude_tokens.active.is_empty() && cfg.exclude_tokens.known.is_empty() {
        for token in cfg.exclude_filter.split(|c: char| c == ',' || c == ';') {
            if let Some(token) = normalize_exclude_token(token) {
                cfg.exclude_tokens.add_token(token);
            }
        }
    }
}

#[cfg(test)]
mod config_tests {
    use super::{
        migrate_legacy_filters, normalize_exclude_token, normalize_extension_token,
        PersistedConfig, ResultView, SortOrder,
    };

    #[test]
    fn missing_config_fields_use_application_defaults() {
        let cfg: PersistedConfig =
            serde_json::from_str(r#"{"pattern":"needle"}"#).unwrap();

        assert_eq!(cfg.pattern, "needle");
        assert!(cfg.respect_ignorefiles);
        assert!(matches!(cfg.sort_order, SortOrder::NameAsc));
        assert_eq!(cfg.ui_scale, 1.4);
        assert!(matches!(cfg.result_view, ResultView::Aligned));
        assert!(cfg.saved_searches.is_empty());
        assert_eq!(cfg.dir_tree.total_dirs(), 0);
    }

    #[test]
    fn legacy_filter_strings_migrate_to_active_known_tokens() {
        let mut cfg: PersistedConfig = serde_json::from_str(
            r#"{"ext_filter":".RS, py;rs","exclude_filter":"target, *.log"}"#,
        )
        .unwrap();
        migrate_legacy_filters(&mut cfg);

        assert_eq!(cfg.ext_tokens.active, ["rs", "py"]);
        assert_eq!(cfg.ext_tokens.known, ["py", "rs"]);
        assert_eq!(cfg.exclude_tokens.active, ["target", "*.log"]);
        assert_eq!(cfg.exclude_tokens.known, ["*.log", "target"]);
    }

    #[test]
    fn migration_does_not_reactivate_intentionally_inactive_tokens() {
        let mut cfg: PersistedConfig = serde_json::from_str(
            r#"{"ext_filter":"rs","ext_tokens":{"active":[],"known":["pdf"]}}"#,
        )
        .unwrap();
        migrate_legacy_filters(&mut cfg);

        assert!(cfg.ext_tokens.active.is_empty());
        assert_eq!(cfg.ext_tokens.known, ["pdf"]);
    }

    #[test]
    fn filter_tokens_normalize_as_expected() {
        assert_eq!(normalize_extension_token(" .PDF ").as_deref(), Some("pdf"));
        assert_eq!(normalize_exclude_token(" *.log ").as_deref(), Some("*.log"));
        assert_eq!(normalize_extension_token(" ... "), None);
        assert_eq!(normalize_exclude_token(" "), None);
    }
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

#[derive(PartialEq, Eq, Clone, Copy, Default)]
pub enum TypeFilter {
    #[default]
    All,
    Files,
    Folders,
}

impl TypeFilter {
    pub const ALL: [TypeFilter; 3] =
        [TypeFilter::All, TypeFilter::Files, TypeFilter::Folders];

    pub fn label(self) -> &'static str {
        match self {
            TypeFilter::All => "All",
            TypeFilter::Files => "Files",
            TypeFilter::Folders => "Folders",
        }
    }
}
