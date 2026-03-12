use crate::ast::symbol::{Parameter, Symbol, Visibility};
use crate::diff::body_diff::BodyDiff;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub enum ChangeKind {
    Added,
    Deleted,
    Renamed {
        old_name: String,
        new_name: String,
    },
    Moved {
        from_file: PathBuf,
        to_file: PathBuf,
    },
    MovedAndModified {
        from_file: PathBuf,
        to_file: PathBuf,
    },
    Extracted {
        from_symbol: String,
        new_symbol: String,
        source_file: PathBuf,
    },
    Inlined {
        from_symbol: String,
        into_symbol: String,
    },
    SignatureChanged {
        details: SignatureDelta,
    },
    BodyChanged,
    VisibilityChanged {
        old: Visibility,
        new: Visibility,
    },
}

impl ChangeKind {
    pub fn label(&self) -> &str {
        match self {
            ChangeKind::Added => "ADD",
            ChangeKind::Deleted => "DEL",
            ChangeKind::Renamed { .. } => "REN",
            ChangeKind::Moved { .. } => "MOV",
            ChangeKind::MovedAndModified { .. } => "M+M",
            ChangeKind::Extracted { .. } => "EXT",
            ChangeKind::Inlined { .. } => "INL",
            ChangeKind::SignatureChanged { .. } => "SIG",
            ChangeKind::BodyChanged => "MOD",
            ChangeKind::VisibilityChanged { .. } => "VIS",
        }
    }

    pub fn short_description(&self) -> String {
        match self {
            ChangeKind::Added => "new symbol".to_string(),
            ChangeKind::Deleted => "removed".to_string(),
            ChangeKind::Renamed { old_name, new_name } => {
                format!("{} -> {}", old_name, new_name)
            }
            ChangeKind::Moved {
                from_file,
                to_file,
            } => format!(
                "{} -> {}",
                from_file.display(),
                to_file.display()
            ),
            ChangeKind::MovedAndModified {
                from_file,
                to_file,
            } => format!(
                "{} -> {} (modified)",
                from_file.display(),
                to_file.display()
            ),
            ChangeKind::Extracted {
                from_symbol,
                new_symbol,
                ..
            } => format!("extracted from {} as {}", from_symbol, new_symbol),
            ChangeKind::Inlined {
                from_symbol,
                into_symbol,
            } => format!("{} inlined into {}", from_symbol, into_symbol),
            ChangeKind::SignatureChanged { details } => {
                let mut parts = Vec::new();
                if !details.params_added.is_empty() {
                    parts.push(format!("+{}params", details.params_added.len()));
                }
                if !details.params_removed.is_empty() {
                    parts.push(format!("-{}params", details.params_removed.len()));
                }
                if details.return_type_changed {
                    parts.push("ret changed".to_string());
                }
                parts.join(", ")
            }
            ChangeKind::BodyChanged => "body modified".to_string(),
            ChangeKind::VisibilityChanged { old, new } => {
                format!("{} -> {}", old, new)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureDelta {
    pub params_added: Vec<Parameter>,
    pub params_removed: Vec<Parameter>,
    pub params_reordered: bool,
    pub return_type_changed: bool,
    pub old_return_type: Option<String>,
    pub new_return_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticChange {
    pub id: usize,
    pub kind: ChangeKind,
    pub old_symbol: Option<Symbol>,
    pub new_symbol: Option<Symbol>,
    pub confidence: f64,
    pub body_diff: Option<BodyDiff>,
    pub related_changes: Vec<usize>,
}

impl SemanticChange {
    pub fn symbol_name(&self) -> &str {
        if let Some(ref s) = self.new_symbol {
            &s.name
        } else if let Some(ref s) = self.old_symbol {
            &s.name
        } else {
            "<unknown>"
        }
    }

    pub fn file_info(&self) -> String {
        match (&self.old_symbol, &self.new_symbol) {
            (Some(old), Some(new)) if old.file_path != new.file_path => {
                format!(
                    "{} -> {}",
                    old.file_path.display(),
                    new.file_path.display()
                )
            }
            (_, Some(new)) => format!("{}", new.file_path.display()),
            (Some(old), _) => format!("{}", old.file_path.display()),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DiffResult {
    pub changes: Vec<SemanticChange>,
    pub old_files: Vec<PathBuf>,
    pub new_files: Vec<PathBuf>,
    pub summary: DiffSummary,
}

#[derive(Debug, Default, Serialize)]
pub struct DiffSummary {
    pub total_changes: usize,
    pub added: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub moved: usize,
    pub extracted: usize,
    pub inlined: usize,
    pub modified: usize,
    pub signature_changed: usize,
}

impl DiffSummary {
    pub fn from_changes(changes: &[SemanticChange]) -> Self {
        let mut s = DiffSummary::default();
        s.total_changes = changes.len();
        for c in changes {
            match &c.kind {
                ChangeKind::Added => s.added += 1,
                ChangeKind::Deleted => s.deleted += 1,
                ChangeKind::Renamed { .. } => s.renamed += 1,
                ChangeKind::Moved { .. } | ChangeKind::MovedAndModified { .. } => s.moved += 1,
                ChangeKind::Extracted { .. } => s.extracted += 1,
                ChangeKind::Inlined { .. } => s.inlined += 1,
                ChangeKind::SignatureChanged { .. } => s.signature_changed += 1,
                ChangeKind::BodyChanged => s.modified += 1,
                ChangeKind::VisibilityChanged { .. } => s.modified += 1,
            }
        }
        s
    }
}
