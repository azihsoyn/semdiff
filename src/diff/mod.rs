pub mod body_diff;
pub mod change;
pub mod classifier;
pub mod cross_file;
pub mod matcher;

use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast;
use crate::ast::symbol::Symbol;
use crate::git::{self, FileStatus, GitRange};
use crate::index;
use change::{ChangeKind, DiffResult, DiffSummary, SemanticChange};
use cross_file::CrossMatchType;

/// Run semantic diff between two directories (or files)
pub fn semantic_diff(old_root: &Path, new_root: &Path) -> Result<DiffResult> {
    let old_files = collect_files(old_root)?;
    let new_files = collect_files(new_root)?;

    let old_symbols = extract_all_symbols(&old_files, old_root)?;
    let new_symbols = extract_all_symbols(&new_files, new_root)?;

    let old_list: Vec<PathBuf> = old_symbols.keys().cloned().collect();
    let new_list: Vec<PathBuf> = new_symbols.keys().cloned().collect();

    run_diff_pipeline(old_symbols, new_symbols, old_list, new_list)
}

fn collect_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut files = Vec::new();
    collect_files_recursive(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('.')
                && name != "target"
                && name != "vendor"
                && name != "node_modules"
            {
                collect_files_recursive(&path, files)?;
            }
        } else if ast::is_supported(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn extract_all_symbols(
    files: &[PathBuf],
    root: &Path,
) -> Result<HashMap<PathBuf, Vec<Symbol>>> {
    let entries: Vec<_> = files
        .par_iter()
        .filter_map(|file| {
            match ast::extract_file_symbols(file) {
                Ok(mut symbols) => {
                    let relpath = file.strip_prefix(root).unwrap_or(file).to_path_buf();
                    for sym in &mut symbols {
                        sym.file_path = relpath.clone();
                    }
                    Some((relpath, symbols))
                }
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {}", file.display(), e);
                    None
                }
            }
        })
        .collect();
    Ok(entries.into_iter().collect())
}

/// Run semantic diff between two git refs
pub fn semantic_diff_git(repo_dir: &Path, range: &GitRange) -> Result<DiffResult> {
    git::validate_ref(repo_dir, &range.old_ref)?;
    git::validate_ref(repo_dir, &range.new_ref)?;

    let changed = git::changed_files(repo_dir, range)?;

    // Separate files to load from old and new refs
    let mut old_to_load: Vec<String> = Vec::new();
    let mut new_to_load: Vec<String> = Vec::new();

    for file in &changed {
        let path = PathBuf::from(&file.path);
        if !ast::is_supported(&path) {
            continue;
        }

        let old_path = match &file.status {
            FileStatus::Renamed { old_path } => old_path.clone(),
            _ => file.path.clone(),
        };

        if file.status != FileStatus::Added {
            old_to_load.push(old_path);
        }
        if file.status != FileStatus::Deleted {
            new_to_load.push(file.path.clone());
        }
    }

    // Batch-load old and new file contents
    let old_refs: Vec<&str> = old_to_load.iter().map(|s| s.as_str()).collect();
    let new_refs: Vec<&str> = new_to_load.iter().map(|s| s.as_str()).collect();

    let old_contents = index::batch_load_files(repo_dir, &range.old_ref, &old_refs)?;
    let new_contents = index::batch_load_files(repo_dir, &range.new_ref, &new_refs)?;

    // Build symbol maps from loaded contents
    let mut old_symbols: HashMap<PathBuf, Vec<Symbol>> = HashMap::new();
    let mut new_symbols: HashMap<PathBuf, Vec<Symbol>> = HashMap::new();
    let mut old_files_list = Vec::new();
    let mut new_files_list = Vec::new();

    let old_parsed: Vec<_> = old_contents
        .par_iter()
        .filter_map(|(path, content)| {
            match ast::extract_symbols_from_bytes(content, path) {
                Ok(mut syms) => {
                    for sym in &mut syms {
                        sym.file_path = path.clone();
                    }
                    Some((path.clone(), syms))
                }
                Err(e) => {
                    eprintln!("Warning: failed to parse old {}: {}", path.display(), e);
                    None
                }
            }
        })
        .collect();

    let new_parsed: Vec<_> = new_contents
        .par_iter()
        .filter_map(|(path, content)| {
            match ast::extract_symbols_from_bytes(content, path) {
                Ok(mut syms) => {
                    for sym in &mut syms {
                        sym.file_path = path.clone();
                    }
                    Some((path.clone(), syms))
                }
                Err(e) => {
                    eprintln!("Warning: failed to parse new {}: {}", path.display(), e);
                    None
                }
            }
        })
        .collect();

    for (path, syms) in old_parsed {
        old_files_list.push(path.clone());
        old_symbols.insert(path, syms);
    }
    for (path, syms) in new_parsed {
        new_files_list.push(path.clone());
        new_symbols.insert(path, syms);
    }

    run_diff_pipeline(old_symbols, new_symbols, old_files_list, new_files_list)
}

/// Common diff pipeline shared between directory and git modes
fn run_diff_pipeline(
    old_symbols: HashMap<PathBuf, Vec<Symbol>>,
    new_symbols: HashMap<PathBuf, Vec<Symbol>>,
    old_files: Vec<PathBuf>,
    new_files: Vec<PathBuf>,
) -> Result<DiffResult> {
    let all_old_paths: std::collections::HashSet<PathBuf> =
        old_symbols.keys().cloned().collect();
    let all_new_paths: std::collections::HashSet<PathBuf> =
        new_symbols.keys().cloned().collect();

    // Parallel within-file matching
    let common_paths: Vec<PathBuf> = all_old_paths.intersection(&all_new_paths).cloned().collect();

    let per_file_results: Vec<_> = common_paths
        .par_iter()
        .map(|relpath| {
            let old_syms = &old_symbols[relpath];
            let new_syms = &new_symbols[relpath];
            let match_result = matcher::match_symbols(old_syms, new_syms);

            let mut file_changes = Vec::new();
            for (oi, ni, confidence) in &match_result.matched {
                let old_sym = &old_syms[*oi];
                let new_sym = &new_syms[*ni];

                if old_sym.body_hash == new_sym.body_hash
                    && old_sym.name == new_sym.name
                    && old_sym.visibility == new_sym.visibility
                {
                    continue;
                }

                let kind = classifier::classify(old_sym, new_sym);
                let body_d = if old_sym.body_hash != new_sym.body_hash {
                    Some(body_diff::body_diff(&old_sym.body_text, &new_sym.body_text))
                } else {
                    None
                };

                file_changes.push((kind, old_sym.clone(), new_sym.clone(), *confidence, body_d));
            }

            let unmatched_old: Vec<Symbol> = match_result
                .unmatched_old
                .iter()
                .map(|&i| old_syms[i].clone())
                .collect();
            let unmatched_new: Vec<Symbol> = match_result
                .unmatched_new
                .iter()
                .map(|&i| new_syms[i].clone())
                .collect();

            (file_changes, unmatched_old, unmatched_new)
        })
        .collect();

    let mut changes: Vec<SemanticChange> = Vec::new();
    let mut change_id = 0;
    let mut all_unmatched_old: Vec<Symbol> = Vec::new();
    let mut all_unmatched_new: Vec<Symbol> = Vec::new();

    for (file_changes, unmatched_old, unmatched_new) in per_file_results {
        for (kind, old_sym, new_sym, confidence, body_d) in file_changes {
            changes.push(SemanticChange {
                id: change_id,
                kind,
                old_symbol: Some(old_sym),
                new_symbol: Some(new_sym),
                confidence,
                body_diff: body_d,
                related_changes: Vec::new(),
            });
            change_id += 1;
        }
        all_unmatched_old.extend(unmatched_old);
        all_unmatched_new.extend(unmatched_new);
    }

    for relpath in all_old_paths.difference(&all_new_paths) {
        if let Some(syms) = old_symbols.get(relpath) {
            all_unmatched_old.extend(syms.clone());
        }
    }

    for relpath in all_new_paths.difference(&all_old_paths) {
        if let Some(syms) = new_symbols.get(relpath) {
            all_unmatched_new.extend(syms.clone());
        }
    }

    let cross_matches =
        cross_file::detect_cross_file_moves(&all_unmatched_old, &all_unmatched_new);

    let mut cross_matched_old = vec![false; all_unmatched_old.len()];
    let mut cross_matched_new = vec![false; all_unmatched_new.len()];

    for m in &cross_matches {
        let old_sym = &all_unmatched_old[m.old_idx];
        let new_sym = &all_unmatched_new[m.new_idx];

        let kind = match m.match_type {
            CrossMatchType::ExactBody => ChangeKind::Moved {
                from_file: old_sym.file_path.clone(),
                to_file: new_sym.file_path.clone(),
            },
            CrossMatchType::NameAndBody | CrossMatchType::SimilarBody => {
                ChangeKind::MovedAndModified {
                    from_file: old_sym.file_path.clone(),
                    to_file: new_sym.file_path.clone(),
                }
            }
            CrossMatchType::Extracted => ChangeKind::Extracted {
                from_symbol: old_sym.name.clone(),
                new_symbol: new_sym.name.clone(),
                source_file: old_sym.file_path.clone(),
            },
            CrossMatchType::Inlined => ChangeKind::Inlined {
                from_symbol: old_sym.name.clone(),
                into_symbol: new_sym.name.clone(),
            },
        };

        let body_d = if old_sym.body_hash != new_sym.body_hash {
            Some(body_diff::body_diff(&old_sym.body_text, &new_sym.body_text))
        } else {
            None
        };

        changes.push(SemanticChange {
            id: change_id,
            kind,
            old_symbol: Some(old_sym.clone()),
            new_symbol: Some(new_sym.clone()),
            confidence: m.confidence,
            body_diff: body_d,
            related_changes: Vec::new(),
        });
        change_id += 1;

        cross_matched_old[m.old_idx] = true;
        cross_matched_new[m.new_idx] = true;
    }

    for (i, sym) in all_unmatched_old.iter().enumerate() {
        if !cross_matched_old[i] {
            changes.push(SemanticChange {
                id: change_id,
                kind: ChangeKind::Deleted,
                old_symbol: Some(sym.clone()),
                new_symbol: None,
                confidence: 1.0,
                body_diff: None,
                related_changes: Vec::new(),
            });
            change_id += 1;
        }
    }

    for (i, sym) in all_unmatched_new.iter().enumerate() {
        if !cross_matched_new[i] {
            changes.push(SemanticChange {
                id: change_id,
                kind: ChangeKind::Added,
                old_symbol: None,
                new_symbol: Some(sym.clone()),
                confidence: 1.0,
                body_diff: None,
                related_changes: Vec::new(),
            });
            change_id += 1;
        }
    }

    link_related_changes(&mut changes);

    let summary = DiffSummary::from_changes(&changes);

    Ok(DiffResult {
        changes,
        old_files,
        new_files,
        summary,
    })
}

fn link_related_changes(changes: &mut Vec<SemanticChange>) {
    let extract_ids: Vec<(usize, String)> = changes
        .iter()
        .filter_map(|c| {
            if let ChangeKind::Extracted { from_symbol, .. } = &c.kind {
                Some((c.id, from_symbol.clone()))
            } else {
                None
            }
        })
        .collect();

    for (extract_id, from_name) in &extract_ids {
        let source_id = changes.iter().find_map(|c| {
            if c.id != *extract_id {
                if let Some(ref sym) = c.old_symbol {
                    if sym.name == *from_name {
                        return Some(c.id);
                    }
                }
            }
            None
        });

        if let Some(sid) = source_id {
            if let Some(c) = changes.iter_mut().find(|c| c.id == *extract_id) {
                c.related_changes.push(sid);
            }
            if let Some(c) = changes.iter_mut().find(|c| c.id == sid) {
                c.related_changes.push(*extract_id);
            }
        }
    }
}
