use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Constant,
    TypeAlias,
    Interface,
    Class,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "fn"),
            SymbolKind::Method => write!(f, "method"),
            SymbolKind::Struct => write!(f, "struct"),
            SymbolKind::Enum => write!(f, "enum"),
            SymbolKind::Trait => write!(f, "trait"),
            SymbolKind::Impl => write!(f, "impl"),
            SymbolKind::Constant => write!(f, "const"),
            SymbolKind::TypeAlias => write!(f, "type"),
            SymbolKind::Interface => write!(f, "interface"),
            SymbolKind::Class => write!(f, "class"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Crate,
    Unknown,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Public => write!(f, "pub"),
            Visibility::Private => write!(f, "private"),
            Visibility::Crate => write!(f, "pub(crate)"),
            Visibility::Unknown => write!(f, ""),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<String>,
}

impl std::fmt::Display for Parameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ty) = &self.type_annotation {
            write!(f, "{}: {}", self.name, ty)
        } else {
            write!(f, "{}", self.name)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub name: String,
    pub qualified_name: String,
    pub file_path: PathBuf,
    pub line_range: (usize, usize),
    pub signature: String,
    pub body_hash: [u8; 32],
    pub body_text: String,
    pub normalized_body: String,
    pub parent: Option<String>,
    pub visibility: Visibility,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<String>,
}

impl Symbol {
    /// Compute body similarity with another symbol (0.0 - 1.0)
    pub fn body_similarity(&self, other: &Symbol) -> f64 {
        if self.body_hash == other.body_hash {
            return 1.0;
        }

        let a = &self.normalized_body;
        let b = &other.normalized_body;

        if a.is_empty() && b.is_empty() {
            return 1.0;
        }
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        let distance = levenshtein(a, b);
        let max_len = a.len().max(b.len());
        1.0 - (distance as f64 / max_len as f64)
    }

    /// Compute name similarity with another symbol (0.0 - 1.0)
    pub fn name_similarity(&self, other: &Symbol) -> f64 {
        if self.name == other.name {
            return 1.0;
        }
        let distance = levenshtein(&self.name, &other.name);
        let max_len = self.name.len().max(other.name.len());
        if max_len == 0 {
            return 1.0;
        }
        1.0 - (distance as f64 / max_len as f64)
    }

    /// Check if signatures differ (ignoring names)
    pub fn signature_differs(&self, other: &Symbol) -> bool {
        let self_sig = self.normalize_signature();
        let other_sig = other.normalize_signature();
        self_sig != other_sig
    }

    fn normalize_signature(&self) -> String {
        // Strip the function name from signature, keep params and return type
        let params: Vec<String> = self
            .parameters
            .iter()
            .map(|p| {
                p.type_annotation
                    .clone()
                    .unwrap_or_else(|| p.name.clone())
            })
            .collect();
        let ret = self.return_type.clone().unwrap_or_default();
        format!("({}) -> {}", params.join(", "), ret)
    }
}

/// Simple Levenshtein distance
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }

    dp[m][n]
}

/// Normalize source body for comparison
/// Strips comments, normalizes whitespace
pub fn normalize_body(source: &str) -> String {
    let mut result = String::new();
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if in_line_comment {
            if chars[i] == '\n' {
                in_line_comment = false;
                result.push(' ');
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            if i + 1 < len && chars[i] == '*' && chars[i + 1] == '/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '/' {
            in_line_comment = true;
            i += 2;
            continue;
        }

        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '*' {
            in_block_comment = true;
            i += 2;
            continue;
        }

        if chars[i].is_whitespace() {
            if !result.ends_with(' ') && !result.is_empty() {
                result.push(' ');
            }
            i += 1;
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    result.trim().to_string()
}
