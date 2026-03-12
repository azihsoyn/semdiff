use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::symbol::Symbol;

#[derive(Debug, Clone, Serialize)]
pub struct SimilarCode {
    pub changed_symbol: String,
    pub similar_symbol: String,
    pub file_path: PathBuf,
    pub line_range: (usize, usize),
    pub similarity: f64,
    pub kind: SimilarityKind,
}

#[derive(Debug, Clone, Serialize)]
pub enum SimilarityKind {
    ExactDuplicate,
    StructurallySimilar,
    NamePattern,
}

impl SimilarityKind {
    pub fn label(&self) -> &str {
        match self {
            SimilarityKind::ExactDuplicate => "EXACT",
            SimilarityKind::StructurallySimilar => "SIMILAR",
            SimilarityKind::NamePattern => "PATTERN",
        }
    }
}

/// Number of hash functions for MinHash
const MINHASH_SIZE: usize = 64;

/// Shingle-based similarity index for fast repo-wide comparison
pub struct SimilarityIndex {
    /// symbol qualified_name -> shingle set
    shingles: HashMap<String, HashSet<u64>>,
    /// symbol qualified_name -> MinHash signature (for fast approximate Jaccard)
    minhashes: HashMap<String, [u64; MINHASH_SIZE]>,
    /// symbol qualified_name -> Symbol
    symbols: HashMap<String, Symbol>,
    /// body hash -> list of symbol names
    hash_index: HashMap<[u8; 32], Vec<String>>,
    /// name stem -> list of symbol names
    stem_index: HashMap<String, Vec<String>>,
}

impl SimilarityIndex {
    /// Build similarity index from all symbols in the repo
    pub fn build(all_symbols: &[Symbol]) -> Self {
        let mut shingles = HashMap::new();
        let mut minhashes = HashMap::new();
        let mut symbols = HashMap::new();
        let mut hash_index: HashMap<[u8; 32], Vec<String>> = HashMap::new();
        let mut stem_index: HashMap<String, Vec<String>> = HashMap::new();

        for sym in all_symbols {
            let key = format!("{}:{}", sym.file_path.display(), sym.qualified_name);

            // Compute shingles
            let shingle_set = compute_shingles(&sym.normalized_body, 4);
            let mh = compute_minhash(&shingle_set);
            minhashes.insert(key.clone(), mh);
            shingles.insert(key.clone(), shingle_set);
            symbols.insert(key.clone(), sym.clone());

            // Hash index for exact matches
            hash_index
                .entry(sym.body_hash)
                .or_default()
                .push(key.clone());

            // Stem index for name pattern matching
            for stem in extract_stems(&sym.name) {
                stem_index.entry(stem).or_default().push(key.clone());
            }
        }

        SimilarityIndex {
            shingles,
            minhashes,
            symbols,
            hash_index,
            stem_index,
        }
    }

    /// Build from pre-computed shingle cache (from index)
    pub fn build_from_cache(
        all_symbols: &[Symbol],
        cache: &[crate::index::SymbolShingleEntry],
    ) -> Self {
        let mut shingles = HashMap::new();
        let mut minhashes = HashMap::new();
        let mut symbols = HashMap::new();
        let mut hash_index: HashMap<[u8; 32], Vec<String>> = HashMap::new();
        let mut stem_index: HashMap<String, Vec<String>> = HashMap::new();

        // Build symbol map
        for sym in all_symbols {
            let key = format!("{}:{}", sym.file_path.display(), sym.qualified_name);
            symbols.insert(key, sym.clone());
        }

        // Load pre-computed data from cache
        for entry in cache {
            let shingle_set: HashSet<u64> = entry.shingles.iter().cloned().collect();
            let mh = compute_minhash(&shingle_set);
            minhashes.insert(entry.key.clone(), mh);
            shingles.insert(entry.key.clone(), shingle_set);

            hash_index
                .entry(entry.body_hash)
                .or_default()
                .push(entry.key.clone());

            for stem in &entry.stems {
                stem_index
                    .entry(stem.clone())
                    .or_default()
                    .push(entry.key.clone());
            }
        }

        SimilarityIndex {
            shingles,
            minhashes,
            symbols,
            hash_index,
            stem_index,
        }
    }

    /// Find symbols similar to the given changed symbols
    pub fn find_similar(
        &self,
        changed_symbols: &[Symbol],
        min_similarity: f64,
    ) -> Vec<SimilarCode> {
        let mut results = Vec::new();

        for changed in changed_symbols {
            let changed_key =
                format!("{}:{}", changed.file_path.display(), changed.qualified_name);

            // Phase 1: Exact duplicates (same body hash, different location)
            if let Some(same_hash) = self.hash_index.get(&changed.body_hash) {
                for key in same_hash {
                    if key == &changed_key {
                        continue;
                    }
                    if let Some(sym) = self.symbols.get(key) {
                        results.push(SimilarCode {
                            changed_symbol: changed.qualified_name.clone(),
                            similar_symbol: sym.qualified_name.clone(),
                            file_path: sym.file_path.clone(),
                            line_range: sym.line_range,
                            similarity: 1.0,
                            kind: SimilarityKind::ExactDuplicate,
                        });
                    }
                }
            }

            // Phase 2: Structurally similar using MinHash for fast approximate Jaccard
            let changed_shingles = compute_shingles(&changed.normalized_body, 4);
            if changed_shingles.len() < 3 {
                continue; // Too small to meaningfully compare
            }

            let changed_mh = compute_minhash(&changed_shingles);
            let mut phase2_count = 0;
            const MAX_SIMILAR_PER_SYMBOL: usize = 10;

            // First pass: collect candidates using MinHash (very fast O(k) per comparison)
            let mut candidates: Vec<(&str, f64)> = Vec::new();
            for (key, other_mh) in &self.minhashes {
                if key == &changed_key {
                    continue;
                }
                // MinHash approximate Jaccard
                let approx_sim = minhash_similarity(&changed_mh, other_mh);
                if approx_sim >= min_similarity - 0.1 {
                    // Use slightly lower threshold to avoid missing matches
                    candidates.push((key.as_str(), approx_sim));
                }
            }

            // Sort candidates by approximate similarity (descending)
            candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            // Second pass: verify top candidates with exact Jaccard
            for (key, _approx) in candidates.iter().take(100) {
                // Check symbol kind early
                let sym = match self.symbols.get(*key) {
                    Some(s) if s.kind == changed.kind && s.body_hash != changed.body_hash => s,
                    _ => continue,
                };

                let other_shingles = match self.shingles.get(*key) {
                    Some(s) if s.len() >= 3 => s,
                    _ => continue,
                };

                let sim = jaccard_similarity(&changed_shingles, other_shingles);
                if sim >= min_similarity && sim < 1.0 {
                    results.push(SimilarCode {
                        changed_symbol: changed.qualified_name.clone(),
                        similar_symbol: sym.qualified_name.clone(),
                        file_path: sym.file_path.clone(),
                        line_range: sym.line_range,
                        similarity: sim,
                        kind: SimilarityKind::StructurallySimilar,
                    });
                    phase2_count += 1;
                    if phase2_count >= MAX_SIMILAR_PER_SYMBOL {
                        break;
                    }
                }
            }

            // Phase 3: Name pattern matches (limited to avoid O(n²) on common stems)
            let changed_stems = extract_stems(&changed.name);
            let mut phase3_found: HashSet<String> = HashSet::new();
            let mut phase3_count = 0;
            const MAX_PATTERN_PER_SYMBOL: usize = 5;
            const MAX_STEM_CANDIDATES: usize = 50;

            for stem in &changed_stems {
                if stem.len() < 4 {
                    // Require longer stems to reduce false positives
                    continue;
                }
                if let Some(keys) = self.stem_index.get(stem) {
                    // Skip very common stems (e.g., "get", "set", "new")
                    if keys.len() > MAX_STEM_CANDIDATES {
                        continue;
                    }
                    for key in keys {
                        if key == &changed_key || phase3_found.contains(key.as_str()) {
                            continue;
                        }
                        if let Some(sym) = self.symbols.get(key) {
                            if sym.kind != changed.kind {
                                continue;
                            }
                            // Skip expensive Levenshtein for very large bodies
                            let max_body =
                                changed.normalized_body.len().max(sym.normalized_body.len());
                            if max_body > 3000 {
                                continue;
                            }
                            let body_sim = changed.body_similarity(sym);
                            if body_sim > 0.3 {
                                phase3_found.insert(key.clone());
                                results.push(SimilarCode {
                                    changed_symbol: changed.qualified_name.clone(),
                                    similar_symbol: sym.qualified_name.clone(),
                                    file_path: sym.file_path.clone(),
                                    line_range: sym.line_range,
                                    similarity: body_sim,
                                    kind: SimilarityKind::NamePattern,
                                });
                                phase3_count += 1;
                                if phase3_count >= MAX_PATTERN_PER_SYMBOL {
                                    break;
                                }
                            }
                        }
                    }
                }
                if phase3_count >= MAX_PATTERN_PER_SYMBOL {
                    break;
                }
            }
        }

        // Sort by similarity descending
        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Deduplicate
        results.dedup_by(|a, b| {
            a.changed_symbol == b.changed_symbol
                && a.similar_symbol == b.similar_symbol
                && a.file_path == b.file_path
        });

        results
    }
}

/// Public wrapper for computing shingles (used by index module)
pub fn compute_shingles_public(text: &str, n: usize) -> HashSet<u64> {
    compute_shingles(text, n)
}

/// Public wrapper for extracting stems (used by index module)
pub fn extract_stems_public(name: &str) -> Vec<String> {
    extract_stems(name)
}

/// Compute 4-gram shingles of a string, hashed to u64
fn compute_shingles(text: &str, n: usize) -> HashSet<u64> {
    let chars: Vec<char> = text.chars().collect();
    let mut shingles = HashSet::new();

    if chars.len() < n {
        return shingles;
    }

    for window in chars.windows(n) {
        let s: String = window.iter().collect();
        let hash = simple_hash(&s);
        shingles.insert(hash);
    }

    shingles
}

/// Simple hash function for shingles
fn simple_hash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3); // FNV prime
    }
    hash
}

/// Jaccard similarity between two sets
fn jaccard_similarity(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

/// Compute MinHash signature from a shingle set
fn compute_minhash(shingles: &HashSet<u64>) -> [u64; MINHASH_SIZE] {
    let mut signature = [u64::MAX; MINHASH_SIZE];
    if shingles.is_empty() {
        return signature;
    }
    for &shingle in shingles {
        for (i, sig) in signature.iter_mut().enumerate() {
            // Use different hash functions by XORing with different seeds
            let h = shingle.wrapping_mul(MINHASH_SEEDS[i]).wrapping_add(MINHASH_SEEDS[i]);
            if h < *sig {
                *sig = h;
            }
        }
    }
    signature
}

/// Approximate Jaccard similarity from MinHash signatures
fn minhash_similarity(a: &[u64; MINHASH_SIZE], b: &[u64; MINHASH_SIZE]) -> f64 {
    let matches = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    matches as f64 / MINHASH_SIZE as f64
}

/// Pre-computed random seeds for MinHash
const MINHASH_SEEDS: [u64; MINHASH_SIZE] = {
    let mut seeds = [0u64; MINHASH_SIZE];
    let mut i = 0;
    let mut h: u64 = 0x517cc1b727220a95;
    while i < MINHASH_SIZE {
        h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seeds[i] = h;
        i += 1;
    }
    seeds
};

/// Extract name stems by splitting on _ and CamelCase boundaries
fn extract_stems(name: &str) -> Vec<String> {
    let mut stems = Vec::new();

    // Split on underscores
    for part in name.split('_') {
        if !part.is_empty() {
            stems.push(part.to_lowercase());
        }
    }

    // Also split on CamelCase boundaries
    let mut current = String::new();
    for ch in name.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            stems.push(current.to_lowercase());
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        stems.push(current.to_lowercase());
    }

    stems.sort();
    stems.dedup();
    stems
}
