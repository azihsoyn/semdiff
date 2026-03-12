use crate::ast::symbol::Symbol;
use std::collections::HashMap;

/// Result of matching symbols within a single file
pub struct MatchResult {
    /// Matched pairs: (old_index, new_index, confidence)
    pub matched: Vec<(usize, usize, f64)>,
    /// Indices of unmatched old symbols
    pub unmatched_old: Vec<usize>,
    /// Indices of unmatched new symbols
    pub unmatched_new: Vec<usize>,
}

/// Match symbols between old and new versions of the same file
pub fn match_symbols(old_symbols: &[Symbol], new_symbols: &[Symbol]) -> MatchResult {
    let mut matched = Vec::new();
    let mut used_old = vec![false; old_symbols.len()];
    let mut used_new = vec![false; new_symbols.len()];

    // Phase 1: Exact qualified name match
    let new_by_qname: HashMap<&str, Vec<usize>> = {
        let mut map: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, sym) in new_symbols.iter().enumerate() {
            map.entry(&sym.qualified_name).or_default().push(i);
        }
        map
    };

    for (oi, old_sym) in old_symbols.iter().enumerate() {
        if let Some(new_indices) = new_by_qname.get(old_sym.qualified_name.as_str()) {
            for &ni in new_indices {
                if !used_new[ni] && !used_old[oi] {
                    let confidence = if old_sym.body_hash == new_symbols[ni].body_hash {
                        1.0
                    } else {
                        0.95
                    };
                    matched.push((oi, ni, confidence));
                    used_old[oi] = true;
                    used_new[ni] = true;
                    break;
                }
            }
        }
    }

    // Phase 2: Body hash match (detect renames within same file)
    let new_by_hash: HashMap<[u8; 32], Vec<usize>> = {
        let mut map: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
        for (i, sym) in new_symbols.iter().enumerate() {
            if !used_new[i] {
                map.entry(sym.body_hash).or_default().push(i);
            }
        }
        map
    };

    for (oi, old_sym) in old_symbols.iter().enumerate() {
        if used_old[oi] {
            continue;
        }
        if let Some(new_indices) = new_by_hash.get(&old_sym.body_hash) {
            for &ni in new_indices {
                if !used_new[ni] && old_sym.kind == new_symbols[ni].kind {
                    matched.push((oi, ni, 0.9));
                    used_old[oi] = true;
                    used_new[ni] = true;
                    break;
                }
            }
        }
    }

    // Phase 3: Name similarity + body similarity for remaining
    let mut candidates: Vec<(usize, usize, f64)> = Vec::new();
    for (oi, old_sym) in old_symbols.iter().enumerate() {
        if used_old[oi] {
            continue;
        }
        for (ni, new_sym) in new_symbols.iter().enumerate() {
            if used_new[ni] {
                continue;
            }
            if old_sym.kind != new_sym.kind {
                continue;
            }
            let name_sim = old_sym.name_similarity(new_sym);
            // Early skip: if name_sim contributes max 0.4, body needs > ~0.17 for score > 0.5
            let body_threshold = (0.5 - name_sim * 0.4).max(0.0) / 0.6;
            let body_sim = old_sym.body_similarity_threshold(new_sym, body_threshold);
            // Weighted combination: name matters more for same-file matching
            let score = name_sim * 0.4 + body_sim * 0.6;
            if score > 0.5 {
                candidates.push((oi, ni, score));
            }
        }
    }

    // Greedy best-match assignment
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    for (oi, ni, confidence) in candidates {
        if !used_old[oi] && !used_new[ni] {
            matched.push((oi, ni, confidence));
            used_old[oi] = true;
            used_new[ni] = true;
        }
    }

    let unmatched_old: Vec<usize> = (0..old_symbols.len()).filter(|&i| !used_old[i]).collect();
    let unmatched_new: Vec<usize> = (0..new_symbols.len()).filter(|&i| !used_new[i]).collect();

    MatchResult {
        matched,
        unmatched_old,
        unmatched_new,
    }
}
