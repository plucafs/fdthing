use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::{collections::HashSet, path::PathBuf};

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
    exts: &[String],
    exclude_globs: &[String],
    dirs_only: bool,
) -> Result<
    (
        SearchStatus,
        Arc<AtomicBool>,
        mpsc::Receiver<ResultEntry>,
    ),
    String,
> {
    let search_paths = prune_overlapping_roots(dir_tree.enabled_paths());

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

    if !exclude_globs.is_empty() {
        let mut ov = OverrideBuilder::new(".");
        for glob in exclude_globs {
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
    let exts = exts.to_vec();

    thread::spawn(move || {
        let mut sent_paths = HashSet::new();
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
                            exts.iter().any(|x| {
                                x.trim()
                                    .trim_start_matches('.')
                                    .eq_ignore_ascii_case(&e)
                            })
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
                        if sent_paths.insert(entry.path.clone()) {
                            let _ = tx.send(entry);
                        }
                    }
                }
            }
        }
    });

    Ok((SearchStatus::Searching, cancel, rx))
}

fn prune_overlapping_roots(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for path in paths {
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(canonical.clone()) {
            unique.push((path, canonical));
        }
    }

    unique
        .iter()
        .enumerate()
        .filter(|(index, (_, candidate))| {
            !unique.iter().enumerate().any(|(other_index, (_, root))| {
                index != &other_index
                    && candidate.as_path() != root.as_path()
                    && candidate.starts_with(root)
            })
        })
        .map(|(_, (path, _))| path.clone())
        .collect()
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::types::{DirTree, SearchDir};

    use super::{prune_overlapping_roots, start_search};

    fn temp_dir() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "fdthing-search-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn overlapping_and_duplicate_roots_are_pruned() {
        let base = temp_dir();
        let root = base.join("root");
        let child = root.join("child");
        let sibling = base.join("sibling");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&sibling).unwrap();

        let roots = prune_overlapping_roots(vec![
            child,
            root.clone(),
            root.clone(),
            sibling.clone(),
        ]);

        assert_eq!(roots, vec![root.clone(), sibling]);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overlapping_search_roots_do_not_duplicate_results() {
        let root = temp_dir();
        let child = root.join("child");
        fs::create_dir_all(&child).unwrap();
        let root_match = root.join("needle-root.txt");
        let child_match = child.join("needle-child.txt");
        fs::write(&root_match, "").unwrap();
        fs::write(&child_match, "").unwrap();

        let dir_tree = DirTree {
            free: vec![
                SearchDir {
                    id: 0,
                    path: root.clone(),
                    enabled: true,
                },
                SearchDir {
                    id: 1,
                    path: child,
                    enabled: true,
                },
            ],
            groups: Vec::new(),
            next_id: 2,
        };
        let (_, _cancel, receiver) = start_search(
            &dir_tree,
            "needle",
            false,
            false,
            false,
            false,
            false,
            &[],
            &[],
            false,
        )
        .unwrap();

        let results: Vec<_> = receiver.into_iter().map(|entry| entry.path).collect();
        let unique: HashSet<_> = results.iter().cloned().collect();
        assert_eq!(results.len(), 2);
        assert_eq!(unique.len(), 2);
        assert!(unique.contains(&root_match));
        assert!(unique.contains(&child_match));
        fs::remove_dir_all(root).unwrap();
    }
}
