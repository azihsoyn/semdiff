use anyhow::{bail, Result};
use clap::Parser;
use std::collections::HashSet;
use std::path::PathBuf;

use semdiff::cli::{Cli, Command, DiffMode, OutputMode};
use semdiff::diff;
use semdiff::git;
use semdiff::index::RepoIndex;
use semdiff::output;
use semdiff::repo;
use semdiff::tui;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle subcommands
    if let Some(cmd) = &cli.command {
        return match cmd {
            Command::Index { git_ref } => run_index(git_ref),
            Command::Symbols { file } => run_symbols(file),
        };
    }

    let mode = cli.diff_mode().map_err(|e| anyhow::anyhow!(e))?;

    // git_info is used later to preload old file contents for TUI
    let (diff_result, repo_analysis, old_root, new_root, git_info) = match mode {
        DiffMode::Git { range_spec } => {
            let cwd = std::env::current_dir()?;
            let repo_dir = git::find_repo_root(&cwd)?;
            let range = git::parse_git_range(&range_spec)?;

            eprintln!("Comparing {} .. {}", range.old_ref, range.new_ref);

            let diff_result = diff::semantic_diff_git(&repo_dir, &range)?;

            let analysis = if cli.repo_analysis {
                Some(repo::analyze_repo_git(
                    &repo_dir,
                    &range.new_ref,
                    &diff_result.changes,
                    cli.impact_depth,
                )?)
            } else {
                None
            };

            // Auto-build index for new_ref if not cached (for next run)
            auto_build_index_background(&repo_dir, &range.new_ref);

            let root = Some(repo_dir.clone());
            let gi = Some((repo_dir, range));
            (diff_result, analysis, root.clone(), root, gi)
        }
        DiffMode::Dirs { old, new } => {
            if !old.exists() {
                bail!("Path does not exist: {}", old.display());
            }
            if !new.exists() {
                bail!("Path does not exist: {}", new.display());
            }

            let diff_result = diff::semantic_diff(&old, &new)?;

            let analysis = if cli.repo_analysis {
                Some(repo::analyze_repo_disk(
                    &new,
                    &diff_result.changes,
                    cli.impact_depth,
                )?)
            } else {
                None
            };

            let old_root = Some(old.canonicalize().unwrap_or(old));
            let new_root = Some(new.canonicalize().unwrap_or(new));
            (diff_result, analysis, old_root, new_root, None)
        }
    };

    // Apply --exclude filters
    let diff_result = if cli.exclude.is_empty() {
        diff_result
    } else {
        use semdiff::diff::change::{DiffResult, DiffSummary};
        let patterns: Vec<glob::Pattern> = cli
            .exclude
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();
        let is_excluded = |path: &str| patterns.iter().any(|pat| pat.matches(path));
        let changes: Vec<_> = diff_result
            .changes
            .into_iter()
            .filter(|c| {
                let file = c.file_info();
                !is_excluded(&file)
            })
            .collect();
        let summary = DiffSummary::from_changes(&changes);
        DiffResult {
            changes,
            old_files: diff_result.old_files,
            new_files: diff_result.new_files,
            summary,
        }
    };

    match cli.output {
        OutputMode::Tui => {
            let mut app = tui::app::App::new(diff_result, cli.llm_review, repo_analysis);
            app.repo_root = new_root;
            app.old_root = old_root;

            // For git mode, preload old/new file contents from git refs
            // Uses batch loading (single git cat-file --batch process) for speed
            if let Some((repo_dir, ref range)) = git_info {
                let t_preload = std::time::Instant::now();

                let mut old_files: HashSet<PathBuf> = HashSet::new();
                let mut new_files: HashSet<PathBuf> = HashSet::new();
                for change in &app.diff_result.changes {
                    if let Some(ref sym) = change.old_symbol {
                        old_files.insert(sym.file_path.clone());
                    }
                    if let Some(ref sym) = change.new_symbol {
                        new_files.insert(sym.file_path.clone());
                    }
                }

                // Batch load old file contents
                let old_paths: Vec<&str> = old_files
                    .iter()
                    .map(|p| p.to_str().unwrap_or(""))
                    .filter(|p| !p.is_empty())
                    .collect();
                if let Ok(old_contents) =
                    semdiff::index::batch_load_files(&repo_dir, &range.old_ref, &old_paths)
                {
                    for (path, content) in old_contents {
                        let cache_key = PathBuf::from("__old__").join(&path);
                        if let Ok(text) = String::from_utf8(content) {
                            app.file_cache.insert(cache_key, text);
                        }
                    }
                }

                // Batch load new file contents
                let new_paths: Vec<&str> = new_files
                    .iter()
                    .map(|p| p.to_str().unwrap_or(""))
                    .filter(|p| !p.is_empty())
                    .collect();
                if let Ok(new_contents) =
                    semdiff::index::batch_load_files(&repo_dir, &range.new_ref, &new_paths)
                {
                    for (path, content) in new_contents {
                        if let Ok(text) = String::from_utf8(content) {
                            app.file_cache.insert(path, text);
                        }
                    }
                }

                eprintln!(
                    "  file preload: {:?} ({} old + {} new files)",
                    t_preload.elapsed(),
                    old_files.len(),
                    new_files.len()
                );
            }

            tui::run_tui(&mut app)?;
        }
        OutputMode::Text => {
            output::text::print_diff(&diff_result);
            if let Some(ref analysis) = repo_analysis {
                output::text::print_repo_analysis(analysis);
            }
        }
        OutputMode::Json => {
            output::json::print_json(&diff_result, repo_analysis.as_ref())?;
        }
    }

    Ok(())
}

fn run_symbols(file: &std::path::Path) -> Result<()> {
    use semdiff::ast;
    let symbols = ast::extract_file_symbols(file)?;
    if symbols.is_empty() {
        eprintln!("No symbols extracted from {}", file.display());
    }
    for sym in &symbols {
        println!(
            "  {:12} {:40} lines {}-{}  (body: {} chars)",
            format!("[{:?}]", sym.kind),
            sym.qualified_name,
            sym.line_range.0,
            sym.line_range.1,
            sym.body_text.len(),
        );
    }
    println!("Total: {} symbols", symbols.len());
    Ok(())
}

fn run_index(git_ref: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_dir = git::find_repo_root(&cwd)?;

    let index = RepoIndex::build_from_git(&repo_dir, git_ref)?;
    index.save(&repo_dir)?;

    eprintln!("Index built successfully.");
    Ok(())
}

/// Auto-build index for a git ref in a background thread if not already cached.
/// This makes subsequent diffs against the same ref much faster.
/// Runs silently — all stderr output is suppressed.
fn auto_build_index_background(repo_dir: &std::path::Path, git_ref: &str) {
    let repo_dir = repo_dir.to_path_buf();
    let git_ref = git_ref.to_string();

    std::thread::spawn(move || {
        // Check if index already matches this ref
        if let Ok(Some(idx)) = RepoIndex::load(&repo_dir) {
            if let Ok(hash) = git::validate_ref(&repo_dir, &git_ref) {
                if idx.commit_hash == hash {
                    return; // Already up to date
                }
            }
        }

        // Build and save silently (redirect stderr to suppress eprintln from build_from_git)
        // We can't suppress eprintln easily, so just build & save
        if let Ok(index) = RepoIndex::build_from_git(&repo_dir, &git_ref) {
            let _ = index.save(&repo_dir);
        }
    });
}
