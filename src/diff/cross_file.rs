use crate::ast::symbol::Symbol;
use rayon::prelude::*;
use std::collections::HashMap;

/// A candidate match between symbols across different files
#[derive(Debug)]
pub struct CrossFileMatch {
    pub old_idx: usize,
    pub new_idx: usize,
    pub confidence: f64,
    pub match_type: CrossMatchType,
}

#[derive(Debug)]
pub enum CrossMatchType {
    ExactBody,      // Same body, different file
    NameAndBody,    // Same name, similar body, different file
    SimilarBody,    // Different name, very similar body, different file
    Extracted,      // Body of new is a subset of old
    Inlined,        // Body of old is a subset of new
}

/// Detect cross-file moves, extractions, and inlines
pub fn detect_cross_file_moves(
    unmatched_old: &[Symbol],
    unmatched_new: &[Symbol],
) -> Vec<CrossFileMatch> {
    use std::time::Instant;
    let t0 = Instant::now();
    let mut candidates: Vec<CrossFileMatch> = Vec::new();

    // Build hash index for new symbols
    let new_by_hash: HashMap<[u8; 32], Vec<usize>> = {
        let mut map: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
        for (i, sym) in unmatched_new.iter().enumerate() {
            map.entry(sym.body_hash).or_default().push(i);
        }
        map
    };

    // Build name index for new symbols
    let new_by_name: HashMap<&str, Vec<usize>> = {
        let mut map: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, sym) in unmatched_new.iter().enumerate() {
            map.entry(&sym.name).or_default().push(i);
        }
        map
    };

    let mut used_old = vec![false; unmatched_old.len()];
    let mut used_new = vec![false; unmatched_new.len()];

    // Phase 1: Exact body hash match across files
    for (oi, old_sym) in unmatched_old.iter().enumerate() {
        if used_old[oi] {
            continue;
        }
        if let Some(new_indices) = new_by_hash.get(&old_sym.body_hash) {
            for &ni in new_indices {
                if used_new[ni] {
                    continue;
                }
                if old_sym.kind == unmatched_new[ni].kind
                    && old_sym.file_path != unmatched_new[ni].file_path
                {
                    candidates.push(CrossFileMatch {
                        old_idx: oi,
                        new_idx: ni,
                        confidence: 0.95,
                        match_type: CrossMatchType::ExactBody,
                    });
                    used_old[oi] = true;
                    used_new[ni] = true;
                    break;
                }
            }
        }
    }

    let t1 = Instant::now();
    eprintln!("      cross P1 (hash): {:?}, {} matches", t1 - t0, candidates.len());

    // Phase 2: Same name, similar body across files
    for (oi, old_sym) in unmatched_old.iter().enumerate() {
        if used_old[oi] {
            continue;
        }
        if let Some(new_indices) = new_by_name.get(old_sym.name.as_str()) {
            let mut best: Option<(usize, f64)> = None;
            for &ni in new_indices {
                if used_new[ni] {
                    continue;
                }
                if old_sym.kind != unmatched_new[ni].kind {
                    continue;
                }
                if old_sym.file_path == unmatched_new[ni].file_path {
                    continue;
                }
                let sim = old_sym.structural_similarity_threshold(&unmatched_new[ni], 0.5);
                if sim > 0.5 {
                    if best.is_none() || sim > best.unwrap().1 {
                        best = Some((ni, sim));
                    }
                }
            }
            if let Some((ni, sim)) = best {
                candidates.push(CrossFileMatch {
                    old_idx: oi,
                    new_idx: ni,
                    confidence: sim * 0.9,
                    match_type: CrossMatchType::NameAndBody,
                });
                used_old[oi] = true;
                used_new[ni] = true;
            }
        }
    }

    let t2 = Instant::now();
    eprintln!("      cross P2 (name+body): {:?}, {} matches", t2 - t1, candidates.len());

    // Phase 3: Different name, very similar body across files (parallel)
    let remaining_old: Vec<usize> = (0..unmatched_old.len())
        .filter(|&i| !used_old[i])
        .collect();
    let remaining_new_count = used_new.iter().filter(|&&u| !u).count();

    // Skip expensive O(n*m) fuzzy matching if there are too many unmatched symbols
    let mut fuzzy_candidates: Vec<CrossFileMatch> =
        if remaining_old.len() * remaining_new_count > 50_000 {
            Vec::new()
        } else {
            remaining_old
                .par_iter()
                .flat_map(|&oi| {
                    let old_sym = &unmatched_old[oi];
                    let mut matches = Vec::new();
                    for (ni, new_sym) in unmatched_new.iter().enumerate() {
                        if used_new[ni] {
                            continue;
                        }
                        if old_sym.kind != new_sym.kind {
                            continue;
                        }
                        if old_sym.file_path == new_sym.file_path {
                            continue;
                        }
                        let body_sim = old_sym.structural_similarity_threshold(new_sym, 0.7);
                        if body_sim > 0.7 {
                            matches.push(CrossFileMatch {
                                old_idx: oi,
                                new_idx: ni,
                                confidence: body_sim * 0.85,
                                match_type: CrossMatchType::SimilarBody,
                            });
                        }
                    }
                    matches
                })
                .collect()
        };

    // Greedy assignment for fuzzy matches
    fuzzy_candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for c in fuzzy_candidates {
        if !used_old[c.old_idx] && !used_new[c.new_idx] {
            used_old[c.old_idx] = true;
            used_new[c.new_idx] = true;
            candidates.push(c);
        }
    }

    let t3 = Instant::now();
    eprintln!("      cross P3 (fuzzy): {:?}, {} total matches", t3 - t2, candidates.len());

    // Phase 4: Extract detection
    // Check if any new symbol's body is largely contained in an old symbol's body
    // Only check large-bodied old symbols (likely sources of extraction)
    let remaining_old_p4: Vec<usize> = (0..unmatched_old.len())
        .filter(|&i| !used_old[i] && unmatched_old[i].normalized_body.len() >= 50)
        .collect();
    let remaining_new_p4: Vec<usize> = (0..unmatched_new.len())
        .filter(|&i| !used_new[i] && unmatched_new[i].normalized_body.len() >= 20)
        .collect();

    // Skip if too many combinations
    if remaining_old_p4.len() * remaining_new_p4.len() <= 100_000 {
        for &oi in &remaining_old_p4 {
            let old_sym = &unmatched_old[oi];
            for &ni in &remaining_new_p4 {
                if used_new[ni] {
                    continue;
                }
                let new_sym = &unmatched_new[ni];
                // Check if new body is a substantial substring of old body
                if old_sym.normalized_body.contains(&new_sym.normalized_body)
                    && new_sym.normalized_body.len() as f64 / old_sym.normalized_body.len() as f64
                        > 0.15
                {
                    candidates.push(CrossFileMatch {
                        old_idx: oi,
                        new_idx: ni,
                        confidence: 0.7,
                        match_type: CrossMatchType::Extracted,
                    });
                    used_new[ni] = true;
                }
            }
        }
    }

    let t4 = Instant::now();
    eprintln!("      cross P4 (extract): {:?}, {} total matches", t4 - t3, candidates.len());

    // Phase 5: Inline detection (reverse of extract)
    let remaining_old_p5: Vec<usize> = (0..unmatched_old.len())
        .filter(|&i| !used_old[i] && unmatched_old[i].normalized_body.len() >= 20)
        .collect();
    let remaining_new_p5: Vec<usize> = (0..unmatched_new.len())
        .filter(|&i| !used_new[i] && unmatched_new[i].normalized_body.len() >= 50)
        .collect();

    if remaining_old_p5.len() * remaining_new_p5.len() <= 100_000 {
        for &oi in &remaining_old_p5 {
            let old_sym = &unmatched_old[oi];
            for &ni in &remaining_new_p5 {
                if used_new[ni] {
                    continue;
                }
                let new_sym = &unmatched_new[ni];
                if new_sym.normalized_body.contains(&old_sym.normalized_body)
                    && old_sym.normalized_body.len() as f64 / new_sym.normalized_body.len() as f64
                        > 0.15
                {
                    candidates.push(CrossFileMatch {
                        old_idx: oi,
                        new_idx: ni,
                        confidence: 0.65,
                        match_type: CrossMatchType::Inlined,
                    });
                    used_old[oi] = true;
                }
            }
        }
    }

    let t5 = Instant::now();
    eprintln!("      cross P5 (inline): {:?}, {} total matches", t5 - t4, candidates.len());
    eprintln!("      cross total: {:?}", t5 - t0);

    candidates
}
