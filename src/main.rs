use anyhow::{bail, Result};
use clap::Parser;

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
        };
    }

    let mode = cli.diff_mode().map_err(|e| anyhow::anyhow!(e))?;

    let (diff_result, repo_analysis, old_root, new_root) = match mode {
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

            // For git mode, both old and new resolve to repo root (files loaded from git)
            let root = Some(repo_dir);
            (diff_result, analysis, root.clone(), root)
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
            (diff_result, analysis, old_root, new_root)
        }
    };

    match cli.output {
        OutputMode::Tui => {
            let mut app = tui::app::App::new(diff_result, cli.llm_review, repo_analysis);
            app.repo_root = new_root;
            app.old_root = old_root;
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

fn run_index(git_ref: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_dir = git::find_repo_root(&cwd)?;

    let index = RepoIndex::build_from_git(&repo_dir, git_ref)?;
    index.save(&repo_dir)?;

    eprintln!("Index built successfully.");
    Ok(())
}
