use serde::Serialize;

use crate::diff::body_diff::{BodyDiff, DiffLineTag};
use crate::diff::change::{ChangeKind, SemanticChange};

/// High-level intent behind a code change, inferred from AST structure and diff patterns.
///
/// This is a novel approach: instead of relying on commit messages or ML models,
/// we analyze the *structural patterns* of changes to classify developer intent.
/// Signals are extracted from:
/// - The type of structural change (rename, move, signature change, etc.)
/// - Diff line patterns (what was added/removed)
/// - Keyword analysis in changed regions
/// - Ratio of added vs removed code
/// - AST similarity metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ChangeIntent {
    /// Bug fix: targeted condition/bounds/null-check changes
    BugFix,
    /// Refactoring: structural reorganization without behavior change
    Refactoring,
    /// New feature: new capabilities or significant new logic
    Feature,
    /// Error handling: added/improved error paths
    ErrorHandling,
    /// Security: input validation, auth checks, sanitization
    Security,
    /// Performance: caching, batching, algorithm optimization
    Performance,
    /// API change: breaking interface modifications
    ApiChange,
    /// Cleanup: dead code removal, formatting, comments
    Cleanup,
}

impl ChangeIntent {
    pub fn label(&self) -> &str {
        match self {
            ChangeIntent::BugFix => "bugfix",
            ChangeIntent::Refactoring => "refactor",
            ChangeIntent::Feature => "feature",
            ChangeIntent::ErrorHandling => "error-handling",
            ChangeIntent::Security => "security",
            ChangeIntent::Performance => "perf",
            ChangeIntent::ApiChange => "api-change",
            ChangeIntent::Cleanup => "cleanup",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            ChangeIntent::BugFix => "BUG",
            ChangeIntent::Refactoring => "REF",
            ChangeIntent::Feature => "FEA",
            ChangeIntent::ErrorHandling => "ERR",
            ChangeIntent::Security => "SEC",
            ChangeIntent::Performance => "PRF",
            ChangeIntent::ApiChange => "API",
            ChangeIntent::Cleanup => "CLN",
        }
    }
}

/// Result of intent classification for a single change
#[derive(Debug, Clone, Serialize)]
pub struct IntentClassification {
    /// Primary intent (highest confidence)
    pub primary: ChangeIntent,
    /// Confidence of primary intent (0.0 - 1.0)
    pub confidence: f64,
    /// Secondary intents with their confidence scores
    pub secondary: Vec<(ChangeIntent, f64)>,
    /// Human-readable signals that contributed to the classification
    pub signals: Vec<String>,
}

/// Classify the intent of a semantic change.
///
/// Algorithm:
/// 1. Score each intent category based on structural signals from ChangeKind
/// 2. Analyze diff line patterns for keyword-based signals
/// 3. Consider code volume ratios (added vs removed)
/// 4. Aggregate scores and pick the highest as primary intent
pub fn classify_intent(change: &SemanticChange) -> IntentClassification {
    let mut scores: Vec<(ChangeIntent, f64)> = vec![
        (ChangeIntent::BugFix, 0.0),
        (ChangeIntent::Refactoring, 0.0),
        (ChangeIntent::Feature, 0.0),
        (ChangeIntent::ErrorHandling, 0.0),
        (ChangeIntent::Security, 0.0),
        (ChangeIntent::Performance, 0.0),
        (ChangeIntent::ApiChange, 0.0),
        (ChangeIntent::Cleanup, 0.0),
    ];
    let mut signals: Vec<String> = Vec::new();

    // Phase 1: Structural signals from ChangeKind
    score_structural(&change.kind, &mut scores, &mut signals);

    // Phase 2: Diff pattern signals
    if let Some(ref diff) = change.body_diff {
        score_diff_patterns(diff, &mut scores, &mut signals);
    }

    // Phase 3: Symbol-level signals
    score_symbol_context(change, &mut scores, &mut signals);

    // Phase 4: AST similarity signals
    score_ast_similarity(change, &mut scores, &mut signals);

    // Normalize and select
    let max_score = scores.iter().map(|(_, s)| *s).fold(0.0f64, f64::max);
    if max_score > 0.0 {
        for (_, s) in &mut scores {
            *s /= max_score;
        }
    }

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let primary = scores[0].0;
    let confidence = scores[0].1;
    let secondary: Vec<(ChangeIntent, f64)> = scores[1..]
        .iter()
        .filter(|(_, s)| *s > 0.3)
        .copied()
        .collect();

    IntentClassification {
        primary,
        confidence,
        secondary,
        signals,
    }
}

/// Phase 1: Score based on structural change type
fn score_structural(
    kind: &ChangeKind,
    scores: &mut Vec<(ChangeIntent, f64)>,
    signals: &mut Vec<String>,
) {
    match kind {
        ChangeKind::Added => {
            add_score(scores, ChangeIntent::Feature, 3.0);
            signals.push("new symbol added".into());
        }
        ChangeKind::Deleted => {
            add_score(scores, ChangeIntent::Cleanup, 2.0);
            add_score(scores, ChangeIntent::Refactoring, 1.0);
            signals.push("symbol removed".into());
        }
        ChangeKind::Renamed { .. } => {
            add_score(scores, ChangeIntent::Refactoring, 4.0);
            signals.push("symbol renamed".into());
        }
        ChangeKind::Moved { .. } => {
            add_score(scores, ChangeIntent::Refactoring, 4.0);
            signals.push("symbol moved to different file".into());
        }
        ChangeKind::MovedAndModified { .. } => {
            add_score(scores, ChangeIntent::Refactoring, 3.0);
            add_score(scores, ChangeIntent::Feature, 1.0);
            signals.push("symbol moved and modified".into());
        }
        ChangeKind::Extracted { .. } => {
            add_score(scores, ChangeIntent::Refactoring, 5.0);
            signals.push("method extraction".into());
        }
        ChangeKind::Inlined { .. } => {
            add_score(scores, ChangeIntent::Refactoring, 4.0);
            add_score(scores, ChangeIntent::Cleanup, 1.0);
            signals.push("method inlined".into());
        }
        ChangeKind::SignatureChanged { details } => {
            add_score(scores, ChangeIntent::ApiChange, 3.0);
            if !details.params_added.is_empty() {
                add_score(scores, ChangeIntent::Feature, 2.0);
                signals.push("parameters added".into());
            }
            if !details.params_removed.is_empty() {
                add_score(scores, ChangeIntent::ApiChange, 2.0);
                signals.push("parameters removed (breaking)".into());
            }
            if details.return_type_changed {
                add_score(scores, ChangeIntent::ApiChange, 2.0);
                signals.push("return type changed".into());
            }
        }
        ChangeKind::BodyChanged => {
            // Body changes need further analysis — don't commit to any intent yet
            add_score(scores, ChangeIntent::BugFix, 1.0);
            add_score(scores, ChangeIntent::Feature, 1.0);
        }
        ChangeKind::VisibilityChanged { .. } => {
            add_score(scores, ChangeIntent::ApiChange, 3.0);
            signals.push("visibility changed".into());
        }
    }
}

/// Phase 2: Score based on diff line content patterns
fn score_diff_patterns(
    diff: &BodyDiff,
    scores: &mut Vec<(ChangeIntent, f64)>,
    signals: &mut Vec<String>,
) {
    let mut added_lines = Vec::new();
    let mut removed_lines = Vec::new();
    let mut equal_count = 0;

    for line in &diff.lines {
        let text: String = line.spans.iter().map(|s| s.text.as_str()).collect();
        let text_lower = text.to_lowercase();
        match line.tag {
            DiffLineTag::Insert => added_lines.push(text_lower),
            DiffLineTag::Delete => removed_lines.push(text_lower),
            DiffLineTag::Equal => equal_count += 1,
        }
    }

    let total_changed = added_lines.len() + removed_lines.len();
    if total_changed == 0 {
        add_score(scores, ChangeIntent::Cleanup, 3.0);
        signals.push("no effective changes (formatting only)".into());
        return;
    }

    // Volume ratio analysis
    let add_ratio = added_lines.len() as f64 / total_changed as f64;
    let change_ratio = total_changed as f64 / (total_changed + equal_count).max(1) as f64;

    // Small, targeted changes → likely bugfix
    if total_changed <= 6 && change_ratio < 0.3 {
        add_score(scores, ChangeIntent::BugFix, 2.0);
        signals.push("small targeted change".into());
    }

    // Mostly additions → likely feature
    if add_ratio > 0.8 && added_lines.len() > 3 {
        add_score(scores, ChangeIntent::Feature, 2.0);
        signals.push("mostly new code added".into());
    }

    // Mostly deletions → likely cleanup
    if add_ratio < 0.2 && removed_lines.len() > 3 {
        add_score(scores, ChangeIntent::Cleanup, 2.0);
        signals.push("mostly code removed".into());
    }

    // Keyword analysis on added lines
    let all_added = added_lines.join(" ");
    let all_removed = removed_lines.join(" ");

    // Bug fix signals
    let bugfix_patterns = [
        ("fix", "fix keyword"),
        ("off by", "off-by-one pattern"),
        ("boundary", "boundary check"),
        ("overflow", "overflow protection"),
        ("underflow", "underflow protection"),
        ("== null", "null check"),
        ("!= null", "null check"),
        ("is_none", "None check"),
        ("is_some", "Some check"),
        ("unwrap_or", "safe unwrap"),
        ("undefined", "undefined check"),
    ];
    for (pattern, signal) in &bugfix_patterns {
        if all_added.contains(pattern) && !all_removed.contains(pattern) {
            add_score(scores, ChangeIntent::BugFix, 1.5);
            signals.push(format!("added {}", signal));
        }
    }

    // Condition change (modified if/match guard) → bugfix signal
    let condition_keywords = ["if ", "else", "match ", "switch ", "case "];
    let cond_added = condition_keywords
        .iter()
        .any(|k| added_lines.iter().any(|l| l.trim_start().starts_with(k)));
    let cond_removed = condition_keywords
        .iter()
        .any(|k| removed_lines.iter().any(|l| l.trim_start().starts_with(k)));
    if cond_added && cond_removed && total_changed <= 8 {
        add_score(scores, ChangeIntent::BugFix, 2.5);
        signals.push("condition/guard modified".into());
    }

    // Error handling signals
    let error_patterns = [
        ("catch", "catch block"),
        ("error", "error handling"),
        ("err ", "error variable"),
        ("result<", "Result type"),
        ("try ", "try block"),
        ("except", "exception handler"),
        ("raise", "exception raising"),
        ("throw", "throw statement"),
        ("anyhow", "anyhow error"),
        ("bail!", "bail macro"),
        ("context(", "error context"),
        (".map_err(", "error mapping"),
    ];
    for (pattern, signal) in &error_patterns {
        if all_added.contains(pattern) && !all_removed.contains(pattern) {
            add_score(scores, ChangeIntent::ErrorHandling, 1.5);
            signals.push(format!("added {}", signal));
        }
    }

    // Security signals
    let security_patterns = [
        ("sanitiz", "input sanitization"),
        ("validat", "input validation"),
        ("escape", "string escaping"),
        ("auth", "authentication"),
        ("permission", "permission check"),
        ("token", "token handling"),
        ("csrf", "CSRF protection"),
        ("xss", "XSS protection"),
        ("inject", "injection prevention"),
        ("encrypt", "encryption"),
        ("hash", "hashing"),
        ("secret", "secret handling"),
        ("credential", "credential handling"),
    ];
    for (pattern, signal) in &security_patterns {
        if all_added.contains(pattern) && !all_removed.contains(pattern) {
            add_score(scores, ChangeIntent::Security, 2.0);
            signals.push(format!("added {}", signal));
        }
    }

    // Performance signals
    let perf_patterns = [
        ("cache", "caching"),
        ("memo", "memoization"),
        ("lazy", "lazy evaluation"),
        ("batch", "batch processing"),
        ("parallel", "parallelization"),
        ("par_iter", "parallel iteration"),
        ("async", "async processing"),
        ("pool", "connection/thread pooling"),
        ("index", "index optimization"),
        ("o(1)", "O(1) optimization"),
        ("o(n)", "O(n) optimization"),
        ("hashmap", "hash-based lookup"),
        ("hashset", "hash-based set"),
        ("buffer", "buffering"),
        ("prefetch", "prefetching"),
    ];
    for (pattern, signal) in &perf_patterns {
        if all_added.contains(pattern) && !all_removed.contains(pattern) {
            add_score(scores, ChangeIntent::Performance, 1.5);
            signals.push(format!("added {}", signal));
        }
    }

    // Constant/literal-only change → likely bugfix (changed threshold, magic number, etc.)
    let only_literals_changed = removed_lines.iter().zip(added_lines.iter()).all(|(r, a)| {
        let r_stripped = strip_literals(r);
        let a_stripped = strip_literals(a);
        r_stripped == a_stripped
    }) && !removed_lines.is_empty()
        && removed_lines.len() == added_lines.len();
    if only_literals_changed {
        add_score(scores, ChangeIntent::BugFix, 2.0);
        signals.push("only literal values changed".into());
    }
}

/// Phase 3: Score based on symbol-level context
fn score_symbol_context(
    change: &SemanticChange,
    scores: &mut Vec<(ChangeIntent, f64)>,
    signals: &mut Vec<String>,
) {
    // Check if symbol name contains intent hints
    let name = change.symbol_name().to_lowercase();

    if name.contains("test") || name.starts_with("test_") {
        add_score(scores, ChangeIntent::Feature, 1.0);
        signals.push("test function".into());
    }

    if name.contains("deprecated") || name.contains("legacy") {
        add_score(scores, ChangeIntent::Cleanup, 2.0);
        signals.push("deprecated/legacy symbol".into());
    }

    if name.contains("validate") || name.contains("sanitize") || name.contains("check") {
        add_score(scores, ChangeIntent::Security, 1.0);
    }

    if name.contains("cache") || name.contains("optimize") || name.contains("fast") {
        add_score(scores, ChangeIntent::Performance, 1.0);
    }

    // If the change has new parameters with default values, likely backward-compatible feature
    if let ChangeKind::SignatureChanged { ref details } = change.kind {
        if !details.params_added.is_empty() && details.params_removed.is_empty() {
            add_score(scores, ChangeIntent::Feature, 2.0);
            add_score(scores, ChangeIntent::ApiChange, -1.0); // less likely breaking
            signals.push("additive parameter change (non-breaking)".into());
        }
    }
}

/// Phase 4: AST similarity signals
fn score_ast_similarity(
    change: &SemanticChange,
    scores: &mut Vec<(ChangeIntent, f64)>,
    signals: &mut Vec<String>,
) {
    let (old, new) = match (&change.old_symbol, &change.new_symbol) {
        (Some(o), Some(n)) => (o, n),
        _ => return,
    };

    if old.body_hash == new.body_hash {
        return;
    }

    let structural_sim = old.structural_similarity(new);
    let text_sim = old.body_similarity(new);

    // High structural similarity but different text → variable rename / literal change (refactoring or bugfix)
    if structural_sim > 0.9 && text_sim < 0.9 {
        add_score(scores, ChangeIntent::Refactoring, 2.0);
        signals.push(format!(
            "AST structure preserved (struct:{:.0}% vs text:{:.0}%)",
            structural_sim * 100.0,
            text_sim * 100.0
        ));
    }

    // Very different structure but similar text → algorithm rewrite
    if structural_sim < 0.5 && text_sim > 0.5 {
        add_score(scores, ChangeIntent::Performance, 1.5);
        add_score(scores, ChangeIntent::BugFix, 1.0);
        signals.push(format!(
            "structural rewrite (struct:{:.0}% vs text:{:.0}%)",
            structural_sim * 100.0,
            text_sim * 100.0
        ));
    }

    // Both low → major rewrite, likely feature
    if structural_sim < 0.3 && text_sim < 0.3 {
        add_score(scores, ChangeIntent::Feature, 2.0);
        signals.push("major rewrite".into());
    }
}

// ============ Helpers ============

fn add_score(scores: &mut Vec<(ChangeIntent, f64)>, intent: ChangeIntent, delta: f64) {
    if let Some((_, s)) = scores.iter_mut().find(|(i, _)| *i == intent) {
        *s = (*s + delta).max(0.0);
    }
}

/// Strip numeric/string literals from a line to check if only values changed
fn strip_literals(line: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip string literals
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            result.push('_');
            i += 1;
            while i < len && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }
        // Skip numeric literals
        if chars[i].is_ascii_digit() {
            result.push('_');
            while i < len && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Classify intents for all changes in a diff result
pub fn classify_all(changes: &[SemanticChange]) -> Vec<IntentClassification> {
    changes.iter().map(classify_intent).collect()
}
