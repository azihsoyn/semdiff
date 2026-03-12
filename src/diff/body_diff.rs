use serde::Serialize;
use similar::{ChangeTag, TextDiff};

/// A span within a diff line, with highlight info
#[derive(Debug, Clone, Serialize)]
pub struct DiffSpan {
    pub text: String,
    /// Whether this span represents a changed portion within the line
    pub highlighted: bool,
}

/// A single line in a diff
#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    pub tag: DiffLineTag,
    pub spans: Vec<DiffSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DiffLineTag {
    Delete,
    Insert,
    Equal,
}

/// Structured body diff with inline word-level highlights
#[derive(Debug, Clone, Serialize)]
pub struct BodyDiff {
    pub lines: Vec<DiffLine>,
}

impl BodyDiff {
    /// Render as plain text (for text output mode)
    pub fn to_plain_text(&self) -> String {
        let mut result = String::new();
        for line in &self.lines {
            let prefix = match line.tag {
                DiffLineTag::Delete => "-",
                DiffLineTag::Insert => "+",
                DiffLineTag::Equal => " ",
            };
            result.push_str(prefix);
            for span in &line.spans {
                result.push_str(&span.text);
            }
            if !result.ends_with('\n') {
                result.push('\n');
            }
        }
        result
    }
}

/// Generate a structured diff with inline word-level highlights
pub fn body_diff(old: &str, new: &str) -> BodyDiff {
    let line_diff = TextDiff::from_lines(old, new);
    let changes: Vec<(ChangeTag, &str)> = line_diff
        .iter_all_changes()
        .map(|c| (c.tag(), c.value()))
        .collect();

    let mut lines = Vec::new();
    let mut i = 0;

    while i < changes.len() {
        match changes[i].0 {
            ChangeTag::Equal => {
                lines.push(DiffLine {
                    tag: DiffLineTag::Equal,
                    spans: vec![DiffSpan {
                        text: changes[i].1.trim_end_matches('\n').to_string(),
                        highlighted: false,
                    }],
                });
                i += 1;
            }
            ChangeTag::Delete => {
                // Collect consecutive deletes
                let del_start = i;
                while i < changes.len() && changes[i].0 == ChangeTag::Delete {
                    i += 1;
                }
                let del_end = i;

                // Collect consecutive inserts that follow
                let ins_start = i;
                while i < changes.len() && changes[i].0 == ChangeTag::Insert {
                    i += 1;
                }
                let ins_end = i;

                let del_lines: Vec<&str> = changes[del_start..del_end]
                    .iter()
                    .map(|(_, v)| *v)
                    .collect();
                let ins_lines: Vec<&str> = changes[ins_start..ins_end]
                    .iter()
                    .map(|(_, v)| *v)
                    .collect();

                // Pair delete/insert lines by similarity (not position)
                let pairs = pair_by_similarity(&del_lines, &ins_lines);
                let mut del_used = vec![false; del_lines.len()];
                let mut ins_used = vec![false; ins_lines.len()];

                // Emit paired lines with word-level diff
                for (di, ii) in &pairs {
                    del_used[*di] = true;
                    ins_used[*ii] = true;
                }

                // Emit all deletes first (unpaired ones fully highlighted, paired with inline diff)
                for j in 0..del_lines.len() {
                    if del_used[j] {
                        // Find paired insert
                        let ii = pairs.iter().find(|(d, _)| *d == j).unwrap().1;
                        let (del_spans, _) =
                            word_level_diff(del_lines[j], ins_lines[ii]);
                        lines.push(DiffLine {
                            tag: DiffLineTag::Delete,
                            spans: del_spans,
                        });
                    } else {
                        lines.push(DiffLine {
                            tag: DiffLineTag::Delete,
                            spans: vec![DiffSpan {
                                text: del_lines[j].trim_end_matches('\n').to_string(),
                                highlighted: true,
                            }],
                        });
                    }
                }

                // Then all inserts
                for j in 0..ins_lines.len() {
                    if ins_used[j] {
                        let di = pairs.iter().find(|(_, i)| *i == j).unwrap().0;
                        let (_, ins_spans) =
                            word_level_diff(del_lines[di], ins_lines[j]);
                        lines.push(DiffLine {
                            tag: DiffLineTag::Insert,
                            spans: ins_spans,
                        });
                    } else {
                        lines.push(DiffLine {
                            tag: DiffLineTag::Insert,
                            spans: vec![DiffSpan {
                                text: ins_lines[j].trim_end_matches('\n').to_string(),
                                highlighted: true,
                            }],
                        });
                    }
                }
            }
            ChangeTag::Insert => {
                // Standalone insert (no preceding delete)
                lines.push(DiffLine {
                    tag: DiffLineTag::Insert,
                    spans: vec![DiffSpan {
                        text: changes[i].1.trim_end_matches('\n').to_string(),
                        highlighted: true,
                    }],
                });
                i += 1;
            }
        }
    }

    BodyDiff { lines }
}

/// Pair delete/insert lines by similarity using greedy best-match
fn pair_by_similarity(del_lines: &[&str], ins_lines: &[&str]) -> Vec<(usize, usize)> {
    if del_lines.is_empty() || ins_lines.is_empty() {
        return Vec::new();
    }

    // Compute similarity matrix
    let mut scores: Vec<(usize, usize, f64)> = Vec::new();
    for (di, del) in del_lines.iter().enumerate() {
        for (ii, ins) in ins_lines.iter().enumerate() {
            let sim = line_similarity(del, ins);
            if sim > 0.3 {
                scores.push((di, ii, sim));
            }
        }
    }

    // Sort by similarity descending (greedy best-match)
    scores.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut del_used = vec![false; del_lines.len()];
    let mut ins_used = vec![false; ins_lines.len()];
    let mut pairs = Vec::new();

    for (di, ii, _sim) in scores {
        if !del_used[di] && !ins_used[ii] {
            del_used[di] = true;
            ins_used[ii] = true;
            pairs.push((di, ii));
        }
    }

    pairs
}

/// Quick line similarity (ratio of common words)
fn line_similarity(a: &str, b: &str) -> f64 {
    let a_words: Vec<&str> = a.split_whitespace().collect();
    let b_words: Vec<&str> = b.split_whitespace().collect();
    if a_words.is_empty() && b_words.is_empty() {
        return 1.0;
    }
    if a_words.is_empty() || b_words.is_empty() {
        return 0.0;
    }
    let common = a_words.iter().filter(|w| b_words.contains(w)).count();
    let total = a_words.len().max(b_words.len());
    common as f64 / total as f64
}

/// Compute word-level diff between two lines, returning spans for each
fn word_level_diff(old_line: &str, new_line: &str) -> (Vec<DiffSpan>, Vec<DiffSpan>) {
    let old_trimmed = old_line.trim_end_matches('\n');
    let new_trimmed = new_line.trim_end_matches('\n');

    let word_diff = TextDiff::from_words(old_trimmed, new_trimmed);

    let mut old_spans = Vec::new();
    let mut new_spans = Vec::new();

    for change in word_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                old_spans.push(DiffSpan {
                    text: change.value().to_string(),
                    highlighted: false,
                });
                new_spans.push(DiffSpan {
                    text: change.value().to_string(),
                    highlighted: false,
                });
            }
            ChangeTag::Delete => {
                old_spans.push(DiffSpan {
                    text: change.value().to_string(),
                    highlighted: true,
                });
            }
            ChangeTag::Insert => {
                new_spans.push(DiffSpan {
                    text: change.value().to_string(),
                    highlighted: true,
                });
            }
        }
    }

    // If the entire line changed, mark all as highlighted
    if old_spans.iter().all(|s| s.highlighted) {
        // nothing to refine, already all highlighted
    }

    (old_spans, new_spans)
}

/// Check if only whitespace/formatting changed
pub fn is_formatting_only(old: &str, new: &str) -> bool {
    let normalize = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    normalize(old) == normalize(new)
}
