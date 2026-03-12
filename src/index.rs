use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::ast;
use crate::ast::call_refs::CallReference;
use crate::ast::symbol::Symbol;
use crate::git;

const INDEX_DIR: &str = ".semdiff";
const INDEX_FILE: &str = "index.bin";

/// Pre-computed shingle data for a symbol, keyed by "file:qualified_name"
#[derive(Debug, Serialize, Deserialize)]
pub struct SymbolShingleEntry {
    pub key: String,
    pub shingles: Vec<u64>,
    pub body_hash: [u8; 32],
    pub stems: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepoIndex {
    pub commit_hash: String,
    pub timestamp: u64,
    pub symbols: Vec<Symbol>,
    pub call_refs: Vec<CallReference>,
    pub file_count: usize,
    pub shingle_cache: Vec<SymbolShingleEntry>,
}

impl RepoIndex {
    /// Build an index from a git ref
    pub fn build_from_git(repo_dir: &Path, git_ref: &str) -> Result<Self> {
        let commit_hash = git::validate_ref(repo_dir, git_ref)?;
        let all_files = git::list_all_files_at_ref(repo_dir, git_ref)?;
        let supported_files: Vec<&str> = all_files
            .iter()
            .filter(|f| ast::is_supported(Path::new(f)))
            .map(|f| f.as_str())
            .collect();

        let file_count = supported_files.len();
        eprintln!("Indexing {} source files at {}...", file_count, git_ref);

        // Batch load file contents using git cat-file --batch
        let file_contents = batch_file_contents(repo_dir, git_ref, &supported_files)?;

        let mut symbols = Vec::new();
        let mut call_refs = Vec::new();

        for (i, (path, source)) in file_contents.iter().enumerate() {
            if (i + 1) % 100 == 0 || i + 1 == file_contents.len() {
                eprint!("\r  Parsing {}/{}...", i + 1, file_contents.len());
            }

            if let Ok(syms) = ast::extract_symbols_from_bytes(source, path) {
                symbols.extend(syms);
            }
            if let Ok(refs) = ast::extract_calls_from_bytes(source, path) {
                call_refs.extend(refs);
            }
        }
        eprintln!();

        // Pre-compute shingles for similarity index
        eprintln!("  Pre-computing similarity data...");
        let shingle_cache = Self::compute_shingle_cache(&symbols);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        eprintln!(
            "  Indexed {} symbols, {} call references, {} shingle entries from {} files",
            symbols.len(),
            call_refs.len(),
            shingle_cache.len(),
            file_count
        );

        Ok(RepoIndex {
            commit_hash,
            timestamp,
            symbols,
            call_refs,
            file_count,
            shingle_cache,
        })
    }

    /// Save index to .semdiff/ directory
    pub fn save(&self, repo_dir: &Path) -> Result<()> {
        let index_dir = repo_dir.join(INDEX_DIR);
        std::fs::create_dir_all(&index_dir)?;

        let index_path = index_dir.join(INDEX_FILE);
        let encoded = bincode::serialize(self).context("Failed to serialize index")?;
        std::fs::write(&index_path, &encoded).context("Failed to write index file")?;

        let size_mb = encoded.len() as f64 / (1024.0 * 1024.0);
        eprintln!(
            "  Saved index to {} ({:.1} MB)",
            index_path.display(),
            size_mb
        );

        // Add .semdiff/ to .gitignore if not already there
        ensure_gitignore(repo_dir)?;

        Ok(())
    }

    /// Load index from .semdiff/ directory
    pub fn load(repo_dir: &Path) -> Result<Option<Self>> {
        let index_path = repo_dir.join(INDEX_DIR).join(INDEX_FILE);
        if !index_path.exists() {
            return Ok(None);
        }

        let data = std::fs::read(&index_path).context("Failed to read index file")?;
        let index: RepoIndex =
            bincode::deserialize(&data).context("Failed to deserialize index (try rebuilding with `semdiff index`)")?;

        Ok(Some(index))
    }

    fn compute_shingle_cache(symbols: &[Symbol]) -> Vec<SymbolShingleEntry> {
        use crate::repo::similarity;

        symbols
            .iter()
            .map(|sym| {
                let key = format!("{}:{}", sym.file_path.display(), sym.qualified_name);
                let shingles: Vec<u64> =
                    similarity::compute_shingles_public(&sym.normalized_body, 4)
                        .into_iter()
                        .collect();
                let stems = similarity::extract_stems_public(&sym.name);
                SymbolShingleEntry {
                    key,
                    shingles,
                    body_hash: sym.body_hash,
                    stems,
                }
            })
            .collect()
    }

    /// Check if the index is up-to-date for a given git ref
    pub fn is_current_for(&self, repo_dir: &Path, git_ref: &str) -> bool {
        match git::validate_ref(repo_dir, git_ref) {
            Ok(hash) => hash == self.commit_hash,
            Err(_) => false,
        }
    }
}

/// Public wrapper for batch loading files (used by repo module)
pub fn batch_load_files(
    repo_dir: &Path,
    git_ref: &str,
    files: &[&str],
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    batch_file_contents(repo_dir, git_ref, files)
}

/// Batch-load file contents using `git cat-file --batch` for performance.
///
/// Uses a writer thread to avoid pipe deadlock when stdin/stdout buffers fill.
fn batch_file_contents(
    repo_dir: &Path,
    git_ref: &str,
    files: &[&str],
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Command, Stdio};

    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(repo_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn git cat-file --batch")?;

    let mut stdin = child.stdin.take().context("Failed to get stdin")?;
    let stdout = child.stdout.take().context("Failed to get stdout")?;

    // Write stdin in a separate thread to avoid deadlock
    let input: Vec<String> = files
        .iter()
        .map(|f| format!("{}:{}", git_ref, f))
        .collect();

    let writer_thread = std::thread::spawn(move || -> Result<()> {
        for line in &input {
            writeln!(stdin, "{}", line)?;
        }
        // stdin is dropped here, closing the pipe
        Ok(())
    });

    // Read stdout in main thread
    let file_paths: Vec<PathBuf> = files.iter().map(|f| PathBuf::from(f)).collect();
    let mut results = Vec::new();
    let mut reader = BufReader::new(stdout);
    let mut header_buf = String::new();
    let mut file_idx = 0;

    while file_idx < file_paths.len() {
        header_buf.clear();
        let n = reader.read_line(&mut header_buf)?;
        if n == 0 {
            break; // EOF
        }

        let header = header_buf.trim_end();

        if header.ends_with("missing") {
            file_idx += 1;
            continue;
        }

        // Parse "<sha> <type> <size>"
        let parts: Vec<&str> = header.split_whitespace().collect();
        if parts.len() < 3 {
            file_idx += 1;
            continue;
        }

        let size: usize = match parts[2].parse() {
            Ok(s) => s,
            Err(_) => {
                file_idx += 1;
                continue;
            }
        };

        // Read exactly `size` bytes of content
        let mut content = vec![0u8; size];
        reader.read_exact(&mut content)?;

        // Read the trailing newline
        let mut newline = [0u8; 1];
        let _ = reader.read_exact(&mut newline);

        results.push((file_paths[file_idx].clone(), content));
        file_idx += 1;
    }

    // Wait for writer thread
    writer_thread
        .join()
        .map_err(|_| anyhow::anyhow!("Writer thread panicked"))??;

    // Wait for child process
    let _ = child.wait();

    Ok(results)
}

/// Ensure .semdiff/ is in .gitignore
fn ensure_gitignore(repo_dir: &Path) -> Result<()> {
    let gitignore_path = repo_dir.join(".gitignore");
    let pattern = ".semdiff/";

    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        if content.lines().any(|l| l.trim() == pattern) {
            return Ok(());
        }
        // Append
        let mut content = content;
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(pattern);
        content.push('\n');
        std::fs::write(&gitignore_path, content)?;
    } else {
        std::fs::write(&gitignore_path, format!("{}\n", pattern))?;
    }

    eprintln!("  Added {} to .gitignore", pattern);
    Ok(())
}
