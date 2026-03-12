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
    /// AST Structure Fingerprint: sorted set of shingle hashes derived from
    /// the DFS traversal of the symbol's AST subtree. Used for O(n+m)
    /// structural similarity comparison instead of O(n*m) Levenshtein.
    #[serde(default)]
    pub ast_fingerprint: Vec<u64>,
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

        // Cap comparison at ~1000 bytes for performance (respecting UTF-8 boundaries)
        let (a_cmp, b_cmp) = if max_len > 1000 {
            let a_end = floor_char_boundary(a, a.len().min(1000));
            let b_end = floor_char_boundary(b, b.len().min(1000));
            (&a[..a_end], &b[..b_end])
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

    /// Compute structural similarity using AST fingerprints (0.0 - 1.0).
    /// Uses Jaccard similarity on sorted shingle sets: O(|A| + |B|).
    ///
    /// This is a novel approach: instead of comparing text (Levenshtein O(n*m)),
    /// we compare the *structure* of the AST. This means:
    /// - Variable renames don't affect similarity
    /// - Formatting/whitespace changes are invisible
    /// - Only structural changes (added/removed statements, changed control flow) matter
    ///
    /// Falls back to text-based comparison if fingerprints are not available.
    pub fn structural_similarity(&self, other: &Symbol) -> f64 {
        if self.body_hash == other.body_hash {
            return 1.0;
        }
        if self.ast_fingerprint.is_empty() || other.ast_fingerprint.is_empty() {
            // Fallback to text-based
            return self.body_similarity_threshold(other, 0.0);
        }
        sorted_jaccard(&self.ast_fingerprint, &other.ast_fingerprint)
    }

    /// Compute structural similarity with early exit if it can't exceed threshold.
    pub fn structural_similarity_threshold(&self, other: &Symbol, threshold: f64) -> f64 {
        if self.body_hash == other.body_hash {
            return 1.0;
        }
        if self.ast_fingerprint.is_empty() || other.ast_fingerprint.is_empty() {
            return self.body_similarity_threshold(other, threshold);
        }

        // Upper bound: |A∩B| / |A∪B| ≤ min(|A|,|B|) / max(|A|,|B|)
        let a_len = self.ast_fingerprint.len();
        let b_len = other.ast_fingerprint.len();
        let upper = a_len.min(b_len) as f64 / a_len.max(b_len).max(1) as f64;
        if upper < threshold {
            return 0.0;
        }

        sorted_jaccard(&self.ast_fingerprint, &other.ast_fingerprint)
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

/// Jaccard similarity on two sorted u64 slices. O(|A| + |B|).
fn sorted_jaccard(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut i = 0;
    let mut j = 0;
    let mut intersection = 0u64;
    let mut union = 0u64;
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            intersection += 1;
            union += 1;
            i += 1;
            j += 1;
        } else if a[i] < b[j] {
            union += 1;
            i += 1;
        } else {
            union += 1;
            j += 1;
        }
    }
    union += (a.len() - i + b.len() - j) as u64;
    if union == 0 {
        return 1.0;
    }
    intersection as f64 / union as f64
}

/// Compute an AST structure fingerprint from a tree-sitter Node.
///
/// Algorithm:
/// 1. DFS traversal of the AST subtree, collecting (node_kind_id, depth) pairs
/// 2. Generate k-shingles (k=3) over the traversal sequence
/// 3. Hash each shingle to u64
/// 4. Return sorted, deduplicated set of shingle hashes
///
/// Key insight: identifiers/literals are replaced with their node *type*,
/// so `x + y` and `a + b` produce identical fingerprints. Only structural
/// changes (different operators, control flow, added statements) differ.
pub fn compute_ast_fingerprint(node: tree_sitter::Node, source: &[u8]) -> Vec<u64> {
    let mut trail: Vec<u16> = Vec::new();
    collect_ast_trail(node, source, 0, &mut trail);

    if trail.len() < 3 {
        return Vec::new();
    }

    // Generate 3-shingles and hash them
    let mut shingles: Vec<u64> = Vec::with_capacity(trail.len() - 2);
    for window in trail.windows(3) {
        // FNV-1a inspired hash for 3 u16 values
        let mut h: u64 = 14695981039346656037;
        for &v in window {
            h ^= v as u64;
            h = h.wrapping_mul(1099511628211);
        }
        shingles.push(h);
    }

    shingles.sort_unstable();
    shingles.dedup();
    shingles
}

/// DFS traversal collecting encoded node types.
/// Named nodes get their kind_id. Anonymous tokens (operators, punctuation)
/// get a special encoding. Identifiers and literals are collapsed to their
/// type (so variable renames don't affect the fingerprint).
fn collect_ast_trail(node: tree_sitter::Node, source: &[u8], depth: u16, trail: &mut Vec<u16>) {
    // Encode: kind_id in lower 10 bits, depth in upper 6 bits (capped at 63)
    let kind_id = node.kind_id();
    let d = depth.min(63);
    let encoded = (d << 10) | (kind_id & 0x3FF);
    trail.push(encoded);

    let cursor = &mut node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_ast_trail(cursor.node(), source, depth + 1, trail);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Find the largest byte index <= `i` that is a valid UTF-8 char boundary.
fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut pos = i;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
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
