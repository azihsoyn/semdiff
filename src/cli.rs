use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "semdiff",
    about = "Semantic diff tool - structure-aware code diff with repo-wide impact analysis",
    version,
    after_help = "EXAMPLES:\n  \
    semdiff                                      Diff HEAD (unstaged changes)\n  \
    semdiff HEAD~3                               Last 3 commits\n  \
    semdiff main..feature                        Branch diff\n  \
    semdiff main..feature --repo-analysis        With impact analysis\n  \
    semdiff --dirs old_dir/ new_dir/             Compare two directories\n  \
    semdiff --dirs old.rs new.rs -o json         Single file, JSON output\n  \
    semdiff index                                Build index at HEAD\n  \
    semdiff index --ref develop                  Build index at a specific ref"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Git range to diff (e.g., HEAD~1..HEAD, main..feature, or a single commit).
    /// If omitted, defaults to HEAD (compares working tree against last commit).
    #[arg(default_value = None)]
    pub range: Option<String>,

    /// Compare two directories or files instead of using git
    #[arg(long, num_args = 2, value_names = ["OLD", "NEW"])]
    pub dirs: Option<Vec<PathBuf>>,

    /// Enable repo-wide impact analysis (call graph + similar code detection)
    #[arg(long)]
    pub repo_analysis: bool,

    /// Max depth for transitive caller analysis
    #[arg(long, default_value = "2")]
    pub impact_depth: usize,

    /// Output mode
    #[arg(short, long, default_value = "tui")]
    pub output: OutputMode,

    /// Enable LLM-powered review
    #[arg(long)]
    pub llm_review: bool,

    /// API key for LLM provider
    #[arg(long, env = "SEMDIFF_API_KEY")]
    pub api_key: Option<String>,

    /// LLM provider (anthropic or openai)
    #[arg(long, default_value = "anthropic")]
    pub llm_provider: String,

    /// LLM model to use
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Build pre-compiled index for faster repo analysis
    Index {
        /// Git ref to index (default: HEAD)
        #[arg(long, default_value = "HEAD")]
        git_ref: String,
    },
}

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputMode {
    Tui,
    Json,
    Text,
}

pub enum DiffMode {
    Git { range_spec: String },
    Dirs { old: PathBuf, new: PathBuf },
}

impl Cli {
    pub fn diff_mode(&self) -> Result<DiffMode, String> {
        if let Some(ref dirs) = self.dirs {
            // --dirs mode
            if dirs.len() != 2 {
                return Err("--dirs requires exactly 2 paths".into());
            }
            Ok(DiffMode::Dirs {
                old: dirs[0].clone(),
                new: dirs[1].clone(),
            })
        } else {
            // Git mode (default)
            // No argument → HEAD~1..HEAD (last commit)
            // Single ref  → <ref>..HEAD  (like git diff <ref>)
            let range_spec = self
                .range
                .clone()
                .unwrap_or_else(|| "HEAD~1..HEAD".to_string());
            Ok(DiffMode::Git { range_spec })
        }
    }
}
