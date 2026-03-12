use std::path::{Path, PathBuf};
use tree_sitter::{Node, Tree};
use serde::{Deserialize, Serialize};

use super::language::Language;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallReference {
    pub caller_name: String,
    pub callee_name: String,
    pub file_path: PathBuf,
    pub line: usize,
}

/// Extract all function call references from a parsed tree
pub fn extract_call_references(
    tree: &Tree,
    source: &[u8],
    path: &Path,
    language: Language,
) -> Vec<CallReference> {
    let root = tree.root_node();
    let mut refs = Vec::new();
    match language {
        Language::Rust => extract_rust_calls(root, source, path, None, &mut refs),
        Language::Go => extract_go_calls(root, source, path, None, &mut refs),
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            extract_ts_calls(root, source, path, None, &mut refs)
        }
        Language::Python => extract_python_calls(root, source, path, None, &mut refs),
    }
    refs
}

fn node_text(node: Node, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).to_string()
}

// ============ Rust ============

fn extract_rust_calls(
    node: Node,
    source: &[u8],
    path: &Path,
    current_fn: Option<&str>,
    refs: &mut Vec<CallReference>,
) {
    match node.kind() {
        "function_item" => {
            let fn_name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source));
            let name = fn_name.as_deref().unwrap_or("<anonymous>");
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_rust_calls(child, source, path, Some(name), refs);
                }
            }
            return;
        }
        "impl_item" => {
            let type_name = node
                .child_by_field_name("type")
                .map(|n| node_text(n, source));
            if let Some(body) = node.child_by_field_name("body") {
                for i in 0..body.child_count() {
                    if let Some(child) = body.child(i) {
                        if child.kind() == "function_item" {
                            let method_name = child
                                .child_by_field_name("name")
                                .map(|n| node_text(n, source));
                            let qualified = match (&type_name, &method_name) {
                                (Some(t), Some(m)) => format!("{}::{}", t, m),
                                (_, Some(m)) => m.clone(),
                                _ => continue,
                            };
                            for j in 0..child.child_count() {
                                if let Some(grandchild) = child.child(j) {
                                    extract_rust_calls(
                                        grandchild,
                                        source,
                                        path,
                                        Some(&qualified),
                                        refs,
                                    );
                                }
                            }
                        } else {
                            extract_rust_calls(child, source, path, current_fn, refs);
                        }
                    }
                }
            }
            return;
        }
        "call_expression" => {
            if let Some(caller) = current_fn {
                let callee = extract_rust_callee(node, source);
                if let Some(callee) = callee {
                    refs.push(CallReference {
                        caller_name: caller.to_string(),
                        callee_name: callee,
                        file_path: path.to_path_buf(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_rust_calls(child, source, path, current_fn, refs);
        }
    }
}

fn extract_rust_callee(node: Node, source: &[u8]) -> Option<String> {
    let func_node = node.child_by_field_name("function")?;
    match func_node.kind() {
        "identifier" => Some(node_text(func_node, source)),
        "scoped_identifier" => Some(node_text(func_node, source)),
        "field_expression" => {
            // e.g., self.method() or obj.method()
            let field = func_node.child_by_field_name("field")?;
            Some(node_text(field, source))
        }
        _ => Some(node_text(func_node, source)),
    }
}

// ============ Go ============

fn extract_go_calls(
    node: Node,
    source: &[u8],
    path: &Path,
    current_fn: Option<&str>,
    refs: &mut Vec<CallReference>,
) {
    match node.kind() {
        "function_declaration" => {
            let fn_name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source));
            let name = fn_name.as_deref().unwrap_or("<anonymous>");
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_go_calls(child, source, path, Some(name), refs);
                }
            }
            return;
        }
        "method_declaration" => {
            let fn_name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source));
            let receiver = node.child_by_field_name("receiver").and_then(|r| {
                for i in 0..r.child_count() {
                    if let Some(child) = r.child(i) {
                        if child.kind() == "parameter_declaration" {
                            if let Some(t) = child.child_by_field_name("type") {
                                return Some(node_text(t, source).trim_start_matches('*').to_string());
                            }
                        }
                    }
                }
                None
            });
            let qualified = match (&receiver, &fn_name) {
                (Some(r), Some(m)) => format!("{}::{}", r, m),
                (_, Some(m)) => m.clone(),
                _ => {
                    // recurse into children anyway
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            extract_go_calls(child, source, path, current_fn, refs);
                        }
                    }
                    return;
                }
            };
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_go_calls(child, source, path, Some(&qualified), refs);
                }
            }
            return;
        }
        "call_expression" => {
            if let Some(caller) = current_fn {
                let callee = extract_go_callee(node, source);
                if let Some(callee) = callee {
                    refs.push(CallReference {
                        caller_name: caller.to_string(),
                        callee_name: callee,
                        file_path: path.to_path_buf(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_go_calls(child, source, path, current_fn, refs);
        }
    }
}

fn extract_go_callee(node: Node, source: &[u8]) -> Option<String> {
    let func_node = node.child_by_field_name("function")?;
    match func_node.kind() {
        "identifier" => Some(node_text(func_node, source)),
        "selector_expression" => {
            let field = func_node.child_by_field_name("field")?;
            Some(node_text(field, source))
        }
        _ => Some(node_text(func_node, source)),
    }
}

// ============ TypeScript / JavaScript ============

fn extract_ts_calls(
    node: Node,
    source: &[u8],
    path: &Path,
    current_fn: Option<&str>,
    refs: &mut Vec<CallReference>,
) {
    match node.kind() {
        "function_declaration" | "method_definition" => {
            let fn_name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source));
            let name = fn_name.as_deref().unwrap_or("<anonymous>");
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_ts_calls(child, source, path, Some(name), refs);
                }
            }
            return;
        }
        "arrow_function" | "function_expression" | "function" => {
            // Use parent variable name if available
            let fn_name = node
                .parent()
                .and_then(|p| p.child_by_field_name("name"))
                .map(|n| node_text(n, source));
            let name = fn_name.as_deref().or(current_fn).unwrap_or("<anonymous>");
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_ts_calls(child, source, path, Some(name), refs);
                }
            }
            return;
        }
        "call_expression" => {
            if let Some(caller) = current_fn {
                let callee = node.child_by_field_name("function").and_then(|f| {
                    match f.kind() {
                        "identifier" => Some(node_text(f, source)),
                        "member_expression" => {
                            f.child_by_field_name("property")
                                .map(|p| node_text(p, source))
                        }
                        _ => Some(node_text(f, source)),
                    }
                });
                if let Some(callee) = callee {
                    refs.push(CallReference {
                        caller_name: caller.to_string(),
                        callee_name: callee,
                        file_path: path.to_path_buf(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_ts_calls(child, source, path, current_fn, refs);
        }
    }
}

// ============ Python ============

fn extract_python_calls(
    node: Node,
    source: &[u8],
    path: &Path,
    current_fn: Option<&str>,
    refs: &mut Vec<CallReference>,
) {
    match node.kind() {
        "function_definition" => {
            let fn_name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source));
            let name = fn_name.as_deref().unwrap_or("<anonymous>");
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_python_calls(child, source, path, Some(name), refs);
                }
            }
            return;
        }
        "call" => {
            if let Some(caller) = current_fn {
                let callee = node.child_by_field_name("function").and_then(|f| {
                    match f.kind() {
                        "identifier" => Some(node_text(f, source)),
                        "attribute" => {
                            f.child_by_field_name("attribute")
                                .map(|a| node_text(a, source))
                        }
                        _ => Some(node_text(f, source)),
                    }
                });
                if let Some(callee) = callee {
                    refs.push(CallReference {
                        caller_name: caller.to_string(),
                        callee_name: callee,
                        file_path: path.to_path_buf(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_python_calls(child, source, path, current_fn, refs);
        }
    }
}
