use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct GitRange {
    pub old_ref: String,
    pub new_ref: String,
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed { old_path: String },
}

/// Parse a git range spec like "main..feature" or a single ref.
///
/// Behaves like `git diff`:
///   - `develop`       → develop..HEAD  (changes since develop)
///   - `main..feature` → main..feature
///   - (no arg / HEAD) → HEAD~1..HEAD   (last commit, handled by caller)
pub fn parse_git_range(spec: &str) -> Result<GitRange> {
    if let Some((old, new)) = spec.split_once("..") {
        Ok(GitRange {
            old_ref: old.to_string(),
            new_ref: new.to_string(),
        })
    } else {
        // Single ref: compare <ref>..HEAD, like `git diff <ref>`
        Ok(GitRange {
            old_ref: spec.to_string(),
            new_ref: "HEAD".to_string(),
        })
    }
}

/// Validate that a git ref exists
pub fn validate_ref(repo_dir: &Path, git_ref: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", git_ref])
        .current_dir(repo_dir)
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        bail!(
            "Invalid git ref '{}': {}",
            git_ref,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get list of changed files between two refs
pub fn changed_files(repo_dir: &Path, range: &GitRange) -> Result<Vec<ChangedFile>> {
    let output = Command::new("git")
        .args([
            "diff",
            "--name-status",
            &format!("{}..{}", range.old_ref, range.new_ref),
        ])
        .current_dir(repo_dir)
        .output()
        .context("Failed to run git diff --name-status")?;

    if !output.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }

        let status_char = parts[0];
        let (status, path) = if status_char.starts_with('R') {
            // Rename: R100\told_path\tnew_path
            if parts.len() >= 3 {
                (
                    FileStatus::Renamed {
                        old_path: parts[1].to_string(),
                    },
                    parts[2].to_string(),
                )
            } else {
                continue;
            }
        } else {
            let status = match status_char {
                "A" => FileStatus::Added,
                "M" => FileStatus::Modified,
                "D" => FileStatus::Deleted,
                _ => continue,
            };
            (status, parts[1].to_string())
        };

        files.push(ChangedFile { path, status });
    }

    Ok(files)
}

/// Get file content at a specific git ref
pub fn file_content_at_ref(repo_dir: &Path, git_ref: &str, file_path: &str) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(["show", &format!("{}:{}", git_ref, file_path)])
        .current_dir(repo_dir)
        .output()
        .context(format!("Failed to get {} at {}", file_path, git_ref))?;

    if !output.status.success() {
        bail!(
            "git show {}:{} failed: {}",
            git_ref,
            file_path,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(output.stdout)
}

/// List all files in the repo at a given ref
pub fn list_all_files_at_ref(repo_dir: &Path, git_ref: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["ls-tree", "-r", "--name-only", git_ref])
        .current_dir(repo_dir)
        .output()
        .context("Failed to run git ls-tree")?;

    if !output.status.success() {
        bail!(
            "git ls-tree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().map(|l| l.to_string()).collect())
}

/// Find the repo root directory
pub fn find_repo_root(start: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .context("Failed to find git repo root")?;

    if !output.status.success() {
        bail!("Not a git repository");
    }

    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// Get the current branch name
pub fn current_branch(repo_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .context("Failed to get current branch")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
