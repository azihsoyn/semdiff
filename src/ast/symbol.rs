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
        self.body_similarity_threshold(other, 0.0)
    }

    /// Compute body similarity, returning 0.0 early if it can't exceed threshold.
    pub fn body_similarity_threshold(&self, other: &Symbol, threshold: f64) -> f64 {
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

        let max_len = a.len().max(b.len());
        let min_len = a.len().min(b.len());

        // Length-based upper bound: similarity can't exceed min_len/max_len
        let length_upper_bound = min_len as f64 / max_len as f64;
        if length_upper_bound < threshold {
            return 0.0;
        }

        // Cap comparison at 1000 chars for performance
        let (a_cmp, b_cmp) = if max_len > 1000 {
            (&a[..a.len().min(1000)], &b[..b.len().min(1000)])
        } else {
            (a.as_str(), b.as_str())
        };

        // Use early-termination Levenshtein with max allowed distance
        let max_distance = ((1.0 - threshold) * max_len as f64) as usize;
        let distance = levenshtein_bounded(a_cmp, b_cmp, max_distance);
        let cmp_max = a_cmp.len().max(b_cmp.len());
        1.0 - (distance as f64 / cmp_max as f64)
    }

    /// Compute name similarity with another symbol (0.0 - 1.0)
    pub fn name_similarity(&self, other: &Symbol) -> f64 {
        if self.name == other.name {
            return 1.0;
        }
        let max_len = self.name.len().max(other.name.len());
        if max_len == 0 {
            return 1.0;
        }
        let distance = levenshtein_bounded(&self.name, &other.name, max_len);
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

/// Levenshtein distance with early termination and O(n) space.
/// Returns `max_dist + 1` if the actual distance would exceed `max_dist`.
fn levenshtein_bounded(a: &str, b: &str, max_dist: usize) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let m = a_bytes.len();
    let n = b_bytes.len();

    // Quick length check
    if m.abs_diff(n) > max_dist {
        return max_dist + 1;
    }

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    // O(n) space: two rows
    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];

    for j in 0..=n {
        prev[j] = j;
    }

    for i in 1..=m {
        curr[0] = i;
        let mut row_min = curr[0];

        for j in 1..=n {
            let cost = if a_bytes[i - 1] == b_bytes[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
            row_min = row_min.min(curr[j]);
        }

        // Early termination: if the minimum value in this row exceeds max_dist,
        // the final result will too
        if row_min > max_dist {
            return max_dist + 1;
        }

        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
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
