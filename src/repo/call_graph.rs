use anyhow::Result;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast;
use crate::ast::call_refs::CallReference;

#[derive(Debug, Clone, Serialize)]
pub struct CallEdge {
    pub caller: String,
    pub callee: String,
    pub file_path: PathBuf,
    pub line: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct CallGraph {
    /// symbol -> symbols it calls
    pub calls: HashMap<String, Vec<CallEdge>>,
    /// symbol -> symbols that call it (reverse index)
    pub callers: HashMap<String, Vec<CallEdge>>,
    pub total_edges: usize,
}

impl CallGraph {
    /// Build call graph from a list of files with their source bytes
    pub fn build(files: &[(PathBuf, Vec<u8>)]) -> Result<Self> {
        let all_refs: Vec<Vec<CallReference>> = files
            .par_iter()
            .filter_map(|(path, source)| {
                if !ast::is_supported(path) {
                    return None;
                }
                ast::extract_calls_from_bytes(source, path).ok()
            })
            .collect();

        let mut graph = CallGraph::default();
        for refs in all_refs {
            for r in refs {
                graph.add_edge(&r);
            }
        }

        Ok(graph)
    }

    /// Build call graph from files on disk
    pub fn build_from_disk(files: &[PathBuf]) -> Result<Self> {
        let mut graph = CallGraph::default();

        for path in files {
            if !ast::is_supported(path) {
                continue;
            }
            let source = match std::fs::read(path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            match ast::extract_calls_from_bytes(&source, path) {
                Ok(refs) => {
                    for r in refs {
                        graph.add_edge(&r);
                    }
                }
                Err(_) => {}
            }
        }

        Ok(graph)
    }

    pub fn add_ref(&mut self, r: &CallReference) {
        self.add_edge(r);
    }

    fn add_edge(&mut self, r: &CallReference) {
        let edge = CallEdge {
            caller: r.caller_name.clone(),
            callee: r.callee_name.clone(),
            file_path: r.file_path.clone(),
            line: r.line,
        };
        self.calls
            .entry(r.caller_name.clone())
            .or_default()
            .push(edge.clone());
        self.callers
            .entry(r.callee_name.clone())
            .or_default()
            .push(edge);
        self.total_edges += 1;
    }

    /// Get all callers of a symbol (direct)
    pub fn callers_of(&self, symbol_name: &str) -> Vec<&CallEdge> {
        // Try exact match first, then try just the short name
        if let Some(edges) = self.callers.get(symbol_name) {
            return edges.iter().collect();
        }

        // Try matching just the function name part (without qualifier)
        let short_name = symbol_name
            .rsplit("::")
            .next()
            .unwrap_or(symbol_name);

        if short_name != symbol_name {
            if let Some(edges) = self.callers.get(short_name) {
                return edges.iter().collect();
            }
        }

        Vec::new()
    }

    /// Get transitive callers up to a given depth
    pub fn transitive_callers(&self, symbol_name: &str, max_depth: usize) -> Vec<(CallEdge, usize)> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut queue: Vec<(String, usize)> = vec![(symbol_name.to_string(), 0)];

        while let Some((name, depth)) = queue.pop() {
            if depth > max_depth {
                continue;
            }
            if !visited.insert(name.clone()) {
                continue;
            }

            for edge in self.callers_of(&name) {
                if depth > 0 || edge.caller != symbol_name {
                    result.push((edge.clone(), depth));
                }
                if depth + 1 <= max_depth {
                    queue.push((edge.caller.clone(), depth + 1));
                }
            }
        }

        result
    }
}
