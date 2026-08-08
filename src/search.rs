use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use regex::Regex;

use crate::types::{DirTree, ResultEntry, SearchStatus, SortOrder, file_name_key};

pub fn start_search(
    dir_tree: &DirTree,
    pattern: &str,
    case_sensitive: bool,
    use_regex: bool,
    show_hidden: bool,
    follow_symlinks: bool,
    respect_ignorefiles: bool,
    ext_filter: &str,
    exclude_filter: &str,
    dirs_only: bool,
) -> Result<
    (
        SearchStatus,
        Arc<AtomicBool>,
        mpsc::Receiver<ResultEntry>,
    ),
    String,
> {
    let search_paths = dir_tree.enabled_paths();

    if search_paths.is_empty() {
        return Err("No directories enabled for search.".into());
    }
    if pattern.trim().is_empty() {
        return Err("Enter a search pattern.".into());
    }

    let regex: Option<Regex> = if use_regex {
        let re_str = if case_sensitive {
            pattern.to_string()
        } else {
            format!("(?i){pattern}")
        };
        match Regex::new(&re_str) {
            Ok(re) => Some(re),
            Err(e) => return Err(format!("Invalid regex: {e}")),
        }
    } else {
        None
    };

    let exts: Vec<String> = ext_filter
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .map(|s| s.trim().trim_start_matches('.'))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();

    let mut builder = WalkBuilder::new(&search_paths[0]);
    for p in &search_paths[1..] {
        builder.add(p);
    }
    builder.hidden(!show_hidden);
    builder.follow_links(follow_symlinks);
    builder.git_ignore(respect_ignorefiles);
    builder.git_global(respect_ignorefiles);
    builder.git_exclude(respect_ignorefiles);
    builder.ignore(respect_ignorefiles);
    builder.max_depth(None);

    let exclude_globs: Vec<&str> = exclude_filter
        .split(|c: char| c == ',' || c == ';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if !exclude_globs.is_empty() {
        let mut ov = OverrideBuilder::new(".");
        for glob in &exclude_globs {
            let _ = ov.add(&format!("!{glob}"));
        }
        if let Ok(ov) = ov.build() {
            builder.overrides(ov);
        }
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let (tx, rx) = mpsc::channel();

    let pattern = pattern.to_string();
    let pattern_lower = pattern.to_lowercase();

    thread::spawn(move || {
        for result in builder.build() {
            if cancel_clone.load(Ordering::Relaxed) {
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

    Ok((SearchStatus::Searching, cancel, rx))
}

pub fn collect_results(
    rx: &mut Option<mpsc::Receiver<ResultEntry>>,
    results: &mut Vec<ResultEntry>,
    _cancel_flag: &mut Option<Arc<AtomicBool>>,
) -> SearchStatus {
    if let Some(ref rx) = rx {
        loop {
            match rx.try_recv() {
                Ok(item) => results.push(item),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return SearchStatus::Done;
                }
            }
        }
        SearchStatus::Searching
    } else {
        SearchStatus::Idle
    }
}

pub fn apply_sort(results: &mut Vec<ResultEntry>, sort_order: SortOrder) {
    use std::cmp::Reverse;

    match sort_order {
        SortOrder::NameAsc => results.sort_by_key(|e| file_name_key(&e.path)),
        SortOrder::NameDesc => {
            results.sort_by_key(|e| Reverse(file_name_key(&e.path)))
        }
        SortOrder::PathAsc => results.sort_by(|a, b| a.path.cmp(&b.path)),
        SortOrder::PathDesc => results.sort_by(|a, b| b.path.cmp(&a.path)),
        SortOrder::SizeAsc => results.sort_by_key(|e| e.size),
        SortOrder::SizeDesc => {
            results.sort_by_key(|e| Reverse(e.size))
        }
        SortOrder::ModifiedAsc => results.sort_by_key(|e| e.modified),
        SortOrder::ModifiedDesc => {
            results.sort_by_key(|e| Reverse(e.modified))
        }
    }
}
