pub mod call_graph;
pub mod impact;
pub mod similarity;

use anyhow::Result;
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::ast;
use crate::ast::symbol::Symbol;
use crate::diff::change::SemanticChange;
use crate::git;
use crate::index::RepoIndex;

use call_graph::CallGraph;
use impact::ImpactAnalysis;
use similarity::SimilarityIndex;

#[derive(Debug, Serialize)]
pub struct RepoAnalysis {
    pub call_graph_edges: usize,
    pub total_repo_symbols: usize,
    pub impact: ImpactAnalysis,
}

/// Analyze the entire repository for impact of the given changes.
///
/// Tries to use the pre-built index if available and current,
/// otherwise falls back to scanning.
pub fn analyze_repo_git(
    repo_dir: &Path,
    git_ref: &str,
    changes: &[SemanticChange],
    impact_depth: usize,
) -> Result<RepoAnalysis> {
    // Try to use cached index
    if let Ok(Some(index)) = RepoIndex::load(repo_dir) {
        if index.is_current_for(repo_dir, git_ref) {
            eprintln!(
                "Using cached index ({} symbols, {} call refs from {} files)",
                index.symbols.len(),
                index.call_refs.len(),
                index.file_count
            );
            return analyze_from_index(&index, changes, impact_depth);
        } else {
            eprintln!("Index is stale (different commit), scanning repository...");
        }
    }

    eprintln!("Scanning repository for impact analysis...");

    // Get all files at the target ref
    let all_files = git::list_all_files_at_ref(repo_dir, git_ref)?;
    let supported_files: Vec<&str> = all_files
        .iter()
        .filter(|f| ast::is_supported(Path::new(f)))
        .map(|f| f.as_str())
        .collect();

    eprintln!("  Parsing {} source files...", supported_files.len());

    // Load all file contents
    let file_contents = crate::index::batch_load_files(repo_dir, git_ref, &supported_files)?;

    analyze_from_contents(&file_contents, changes, impact_depth)
}

/// Analyze using pre-built index data
fn analyze_from_index(
    index: &RepoIndex,
    changes: &[SemanticChange],
    impact_depth: usize,
) -> Result<RepoAnalysis> {
    eprintln!("  Building call graph from index...");
    let mut call_graph = CallGraph::default();
    for r in &index.call_refs {
        call_graph.add_ref(r);
    }
    let total_edges = call_graph.total_edges;

    eprintln!("  Building similarity index from cache...");
    let total_symbols = index.symbols.len();
    let sim_index = SimilarityIndex::build_from_cache(&index.symbols, &index.shingle_cache);

    let changed_symbols: Vec<Symbol> = changes
        .iter()
        .filter_map(|c| {
            c.new_symbol
                .as_ref()
                .or(c.old_symbol.as_ref())
                .cloned()
        })
        .collect();

    let similar_code = sim_index.find_similar(&changed_symbols, 0.6);

    eprintln!("  Computing impact analysis...");
    let impact = impact::analyze_impact(changes, &call_graph, &similar_code, impact_depth);

    eprintln!(
        "  Done: {} call edges, {} symbols, {} affected callers, {} similar code, {} warnings",
        total_edges,
        total_symbols,
        impact.affected_callers.len(),
        impact.similar_code.len(),
        impact.pattern_warnings.len(),
    );

    Ok(RepoAnalysis {
        call_graph_edges: total_edges,
        total_repo_symbols: total_symbols,
        impact,
    })
}

/// Analyze repository from directory on disk
pub fn analyze_repo_disk(
    root: &Path,
    changes: &[SemanticChange],
    impact_depth: usize,
) -> Result<RepoAnalysis> {
    eprintln!("Scanning directory for impact analysis...");

    let files = collect_all_files(root)?;
    let supported: Vec<&PathBuf> = files.iter().filter(|f| ast::is_supported(f)).collect();

    eprintln!("  Parsing {} source files...", supported.len());

    let mut file_contents: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for path in &supported {
        match std::fs::read(path) {
            Ok(content) => {
                let relpath = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                file_contents.push((relpath, content));
            }
            Err(_) => {}
        }
    }

    analyze_from_contents(&file_contents, changes, impact_depth)
}

fn analyze_from_contents(
    file_contents: &[(PathBuf, Vec<u8>)],
    changes: &[SemanticChange],
    impact_depth: usize,
) -> Result<RepoAnalysis> {
    // Build call graph
    eprintln!("  Building call graph...");
    let call_graph = CallGraph::build(file_contents)?;
    let total_edges = call_graph.total_edges;

    // Extract all symbols for similarity analysis (parallel)
    eprintln!("  Building similarity index...");
    let per_file_syms: Vec<Vec<Symbol>> = file_contents
        .par_iter()
        .filter_map(|(path, source)| ast::extract_symbols_from_bytes(source, path).ok())
        .collect();
    let all_symbols: Vec<Symbol> = per_file_syms.into_iter().flatten().collect();
    let total_symbols = all_symbols.len();

    // Build similarity index
    let sim_index = SimilarityIndex::build(&all_symbols);

    // Get changed symbols for similarity search
    let changed_symbols: Vec<Symbol> = changes
        .iter()
        .filter_map(|c| {
            c.new_symbol
                .as_ref()
                .or(c.old_symbol.as_ref())
                .cloned()
        })
        .collect();

    let similar_code = sim_index.find_similar(&changed_symbols, 0.6);

    eprintln!("  Computing impact analysis...");
    let impact = impact::analyze_impact(changes, &call_graph, &similar_code, impact_depth);

    eprintln!(
        "  Done: {} call edges, {} symbols, {} affected callers, {} similar code, {} warnings",
        total_edges,
        total_symbols,
        impact.affected_callers.len(),
        impact.similar_code.len(),
        impact.pattern_warnings.len(),
    );

    Ok(RepoAnalysis {
        call_graph_edges: total_edges,
        total_repo_symbols: total_symbols,
        impact,
    })
}

fn collect_all_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_recursive(dir, &mut files)?;
    Ok(files)
}

fn collect_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
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
                collect_recursive(&path, files)?;
            }
        } else {
            files.push(path);
        }
    }
    Ok(())
}
