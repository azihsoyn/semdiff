use std::path::Path;
use tree_sitter::{Node, Tree};

use super::language::Language;
use super::symbol::{compute_ast_fingerprint, normalize_body, Parameter, Symbol, SymbolKind, Visibility};

/// Extract all symbols from a parsed tree
pub fn extract_symbols(tree: &Tree, source: &[u8], path: &Path, language: Language) -> Vec<Symbol> {
    let root = tree.root_node();
    let mut symbols = Vec::new();
    extract_from_node(root, source, path, language, None, &mut symbols);
    symbols
}

fn extract_from_node(
    node: Node,
    source: &[u8],
    path: &Path,
    language: Language,
    parent: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    match language {
        Language::Rust => extract_rust_node(node, source, path, parent, symbols),
        Language::Go => extract_go_node(node, source, path, parent, symbols),
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            extract_ts_node(node, source, path, parent, symbols)
        }
        Language::Python => extract_python_node(node, source, path, parent, symbols),
    }
}

fn extract_rust_node(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    let kind = node.kind();

    match kind {
        "function_item" | "function_signature_item" => {
            if let Some(sym) = extract_rust_function(node, source, path, parent) {
                symbols.push(sym);
            }
        }
        "struct_item" => {
            if let Some(sym) = extract_rust_type_def(node, source, path, SymbolKind::Struct) {
                let name = sym.name.clone();
                symbols.push(sym);
                // Extract children within struct
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        extract_rust_node(child, source, path, Some(&name), symbols);
                    }
                }
                return;
            }
        }
        "enum_item" => {
            if let Some(sym) = extract_rust_type_def(node, source, path, SymbolKind::Enum) {
                symbols.push(sym);
            }
        }
        "trait_item" => {
            if let Some(sym) = extract_rust_type_def(node, source, path, SymbolKind::Trait) {
                let name = sym.name.clone();
                symbols.push(sym);
                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_rust_node(child, source, path, Some(&name), symbols);
                        }
                    }
                }
                return;
            }
        }
        "impl_item" => {
            let impl_name = extract_rust_impl_name(node, source);
            if let Some(body) = node.child_by_field_name("body") {
                for i in 0..body.child_count() {
                    if let Some(child) = body.child(i) {
                        extract_rust_node(
                            child,
                            source,
                            path,
                            Some(impl_name.as_deref().unwrap_or("impl")),
                            symbols,
                        );
                    }
                }
            }
            return;
        }
        "const_item" | "static_item" => {
            if let Some(sym) = extract_rust_const(node, source, path, parent) {
                symbols.push(sym);
            }
        }
        "type_item" => {
            if let Some(sym) = extract_rust_type_def(node, source, path, SymbolKind::TypeAlias) {
                symbols.push(sym);
            }
        }
        _ => {}
    }

    // Recurse into children for top-level nodes
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_rust_node(child, source, path, parent, symbols);
        }
    }
}

fn extract_rust_function(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);

    let visibility = extract_rust_visibility(node, source);

    let parameters = extract_rust_parameters(node, source);
    let return_type = extract_rust_return_type(node, source);

    let body_text = node_text(node, source);
    let signature = build_signature(&name, &parameters, &return_type);

    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    let qualified_name = if let Some(p) = parent {
        format!("{}::{}", p, name)
    } else {
        name.clone()
    };

    let sym_kind = if parent.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    Some(Symbol {
        kind: sym_kind,
        name,
        qualified_name,
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature,
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: parent.map(String::from),
        visibility,
        parameters,
        return_type,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_rust_type_def(
    node: Node,
    source: &[u8],
    path: &Path,
    kind: SymbolKind,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    let visibility = extract_rust_visibility(node, source);
    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    Some(Symbol {
        kind,
        name: name.clone(),
        qualified_name: name,
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature: String::new(),
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: None,
        visibility,
        parameters: Vec::new(),
        return_type: None,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_rust_const(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    let visibility = extract_rust_visibility(node, source);
    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    let qualified_name = if let Some(p) = parent {
        format!("{}::{}", p, name)
    } else {
        name.clone()
    };

    Some(Symbol {
        kind: SymbolKind::Constant,
        name,
        qualified_name,
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature: String::new(),
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: parent.map(String::from),
        visibility,
        parameters: Vec::new(),
        return_type: None,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_rust_impl_name(node: Node, source: &[u8]) -> Option<String> {
    // impl Type { ... } or impl Trait for Type { ... }
    let type_node = node.child_by_field_name("type")?;
    Some(node_text(type_node, source))
}

fn extract_rust_visibility(node: Node, source: &[u8]) -> Visibility {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "visibility_modifier" {
                let text = node_text(child, source);
                return if text.contains("crate") {
                    Visibility::Crate
                } else {
                    Visibility::Public
                };
            }
        }
    }
    Visibility::Private
}

fn extract_rust_parameters(node: Node, source: &[u8]) -> Vec<Parameter> {
    let mut params = Vec::new();
    if let Some(params_node) = node.child_by_field_name("parameters") {
        for i in 0..params_node.child_count() {
            if let Some(child) = params_node.child(i) {
                match child.kind() {
                    "parameter" => {
                        let name = child
                            .child_by_field_name("pattern")
                            .map(|n| node_text(n, source))
                            .unwrap_or_default();
                        let type_ann = child
                            .child_by_field_name("type")
                            .map(|n| node_text(n, source));
                        params.push(Parameter {
                            name,
                            type_annotation: type_ann,
                        });
                    }
                    "self_parameter" => {
                        params.push(Parameter {
                            name: node_text(child, source),
                            type_annotation: None,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    params
}

fn extract_rust_return_type(node: Node, source: &[u8]) -> Option<String> {
    node.child_by_field_name("return_type")
        .map(|n| node_text(n, source))
}

// ============ Go support ============

fn extract_go_node(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    let kind = node.kind();

    match kind {
        "function_declaration" => {
            if let Some(sym) = extract_go_function(node, source, path) {
                symbols.push(sym);
            }
        }
        "method_declaration" => {
            if let Some(sym) = extract_go_method(node, source, path) {
                symbols.push(sym);
            }
        }
        "type_declaration" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "type_spec" {
                        if let Some(sym) = extract_go_type_spec(child, source, path) {
                            symbols.push(sym);
                        }
                    }
                }
            }
        }
        "const_declaration" | "var_declaration" => {
            // Extract const/var specs
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "const_spec" || child.kind() == "var_spec" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let name = node_text(name_node, source);
                            let body_text = node_text(child, source);
                            let normalized = normalize_body(&body_text);
                            let body_hash = blake3::hash(normalized.as_bytes()).into();
                            symbols.push(Symbol {
                                kind: SymbolKind::Constant,
                                name: name.clone(),
                                qualified_name: name,
                                file_path: path.to_path_buf(),
                                line_range: (
                                    child.start_position().row + 1,
                                    child.end_position().row + 1,
                                ),
                                signature: String::new(),
                                body_hash,
                                body_text,
                                normalized_body: normalized,
                                parent: None,
                                visibility: Visibility::Unknown,
                                parameters: Vec::new(),
                                return_type: None,
                                ast_fingerprint: compute_ast_fingerprint(child, source),
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_go_node(child, source, path, parent, symbols);
        }
    }
}

fn extract_go_function(node: Node, source: &[u8], path: &Path) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);

    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    let parameters = extract_go_parameters(node, source);
    let return_type = extract_go_return_type(node, source);
    let signature = build_signature(&name, &parameters, &return_type);

    let visibility = if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    Some(Symbol {
        kind: SymbolKind::Function,
        name: name.clone(),
        qualified_name: name,
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature,
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: None,
        visibility,
        parameters,
        return_type,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_go_method(node: Node, source: &[u8], path: &Path) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);

    // Get receiver type
    let receiver = node
        .child_by_field_name("receiver")
        .and_then(|r| {
            // parameter_list -> parameter_declaration -> type
            for i in 0..r.child_count() {
                if let Some(child) = r.child(i) {
                    if child.kind() == "parameter_declaration" {
                        if let Some(type_node) = child.child_by_field_name("type") {
                            return Some(node_text(type_node, source));
                        }
                        // If no named type field, get the last child that looks like a type
                        let text = node_text(child, source);
                        return Some(text);
                    }
                }
            }
            None
        });

    let receiver_name = receiver
        .as_ref()
        .map(|r| r.trim_start_matches('*').to_string())
        .unwrap_or_else(|| "?".to_string());

    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    let parameters = extract_go_parameters(node, source);
    let return_type = extract_go_return_type(node, source);
    let signature = build_signature(&name, &parameters, &return_type);

    let visibility = if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    Some(Symbol {
        kind: SymbolKind::Method,
        name: name.clone(),
        qualified_name: format!("{}::{}", receiver_name, name),
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature,
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: Some(receiver_name),
        visibility,
        parameters,
        return_type,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_go_type_spec(node: Node, source: &[u8], path: &Path) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);

    let kind = if let Some(type_node) = node.child_by_field_name("type") {
        match type_node.kind() {
            "struct_type" => SymbolKind::Struct,
            "interface_type" => SymbolKind::Interface,
            _ => SymbolKind::TypeAlias,
        }
    } else {
        SymbolKind::TypeAlias
    };

    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    let visibility = if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        Visibility::Public
    } else {
        Visibility::Private
    };

    Some(Symbol {
        kind,
        name: name.clone(),
        qualified_name: name,
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature: String::new(),
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: None,
        visibility,
        parameters: Vec::new(),
        return_type: None,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_go_parameters(node: Node, source: &[u8]) -> Vec<Parameter> {
    let mut params = Vec::new();
    if let Some(params_node) = node.child_by_field_name("parameters") {
        for i in 0..params_node.child_count() {
            if let Some(child) = params_node.child(i) {
                if child.kind() == "parameter_declaration" {
                    let name = child
                        .child_by_field_name("name")
                        .map(|n| node_text(n, source))
                        .unwrap_or_default();
                    let type_ann = child
                        .child_by_field_name("type")
                        .map(|n| node_text(n, source));
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
        }
    }
    params
}

fn extract_go_return_type(node: Node, source: &[u8]) -> Option<String> {
    node.child_by_field_name("result")
        .map(|n| node_text(n, source))
}

// ============ TypeScript / JavaScript ============

fn extract_ts_node(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    let kind = node.kind();

    match kind {
        "function_declaration" => {
            if let Some(sym) = extract_ts_function(node, source, path, parent) {
                symbols.push(sym);
            }
            return; // don't recurse into function body for top-level extraction
        }
        "export_statement" => {
            // Check if this is `export default <expr>` — extract as a symbol
            if let Some(sym) = extract_ts_export_default(node, source, path) {
                symbols.push(sym);
            }
            // Always recurse into exported declarations (including inside default exports)
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_ts_node(child, source, path, parent, symbols);
                }
            }
            return;
        }
        "lexical_declaration" => {
            // const foo = () => {} or const FOO = ...
            extract_ts_lexical(node, source, path, parent, symbols);
            return;
        }
        "class_declaration" => {
            if let Some(sym) = extract_ts_class(node, source, path) {
                let name = sym.name.clone();
                symbols.push(sym);
                // Extract methods
                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            if child.kind() == "method_definition" {
                                if let Some(msym) =
                                    extract_ts_method(child, source, path, &name)
                                {
                                    symbols.push(msym);
                                }
                            }
                        }
                    }
                }
                return;
            }
        }
        "interface_declaration" | "type_alias_declaration" => {
            if let Some(sym) = extract_ts_type_decl(node, source, path, kind) {
                symbols.push(sym);
            }
            return;
        }
        "enum_declaration" => {
            if let Some(sym) = extract_ts_type_decl(node, source, path, "enum_declaration") {
                symbols.push(sym);
            }
            return;
        }
        // Don't recurse into function bodies — local variables are not top-level symbols
        "arrow_function" | "function_expression" | "function" => {
            return;
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_ts_node(child, source, path, parent, symbols);
        }
    }
}

fn extract_ts_function(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);

    let parameters = extract_ts_parameters(node, source);
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| node_text(n, source));

    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();
    let signature = build_signature(&name, &parameters, &return_type);

    let visibility = detect_ts_export(node, source);

    let qualified_name = if let Some(p) = parent {
        format!("{}.{}", p, name)
    } else {
        name.clone()
    };

    let sym_kind = if parent.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    Some(Symbol {
        kind: sym_kind,
        name,
        qualified_name,
        file_path: path.to_path_buf(),
        line_range: (start_line_with_comment(node), node.end_position().row + 1),
        signature,
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: parent.map(String::from),
        visibility,
        parameters,
        return_type,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_ts_lexical(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        if child.kind() != "variable_declarator" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let name = node_text(name_node, source);

        let Some(value_node) = child.child_by_field_name("value") else {
            continue;
        };

        let is_function = matches!(
            value_node.kind(),
            "arrow_function" | "function_expression" | "function"
        );

        let body_text = node_text(node, source);
        let normalized = normalize_body(&body_text);
        let body_hash = blake3::hash(normalized.as_bytes()).into();

        let (sym_kind, parameters, return_type) = if is_function {
            let params = extract_ts_parameters(value_node, source);
            let ret = value_node
                .child_by_field_name("return_type")
                .map(|n| node_text(n, source));
            (SymbolKind::Function, params, ret)
        } else {
            (SymbolKind::Constant, Vec::new(), None)
        };

        let signature = if is_function {
            build_signature(&name, &parameters, &return_type)
        } else {
            String::new()
        };

        let visibility = detect_ts_export(node, source);

        let qualified_name = if let Some(p) = parent {
            format!("{}.{}", p, name)
        } else {
            name.clone()
        };

        symbols.push(Symbol {
            kind: sym_kind,
            name,
            qualified_name,
            file_path: path.to_path_buf(),
            line_range: (start_line_with_comment(node), node.end_position().row + 1),
            signature,
            body_hash,
            body_text,
            normalized_body: normalized,
            parent: parent.map(String::from),
            visibility,
            parameters,
            return_type,
            ast_fingerprint: compute_ast_fingerprint(node, source),
        });
    }
}

fn extract_ts_class(node: Node, source: &[u8], path: &Path) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();
    let visibility = detect_ts_export(node, source);

    Some(Symbol {
        kind: SymbolKind::Class,
        name: name.clone(),
        qualified_name: name,
        file_path: path.to_path_buf(),
        line_range: (start_line_with_comment(node), node.end_position().row + 1),
        signature: String::new(),
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: None,
        visibility,
        parameters: Vec::new(),
        return_type: None,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_ts_method(
    node: Node,
    source: &[u8],
    path: &Path,
    class_name: &str,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);

    let parameters = extract_ts_parameters(node, source);
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| node_text(n, source));

    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();
    let signature = build_signature(&name, &parameters, &return_type);

    Some(Symbol {
        kind: SymbolKind::Method,
        name: name.clone(),
        qualified_name: format!("{}.{}", class_name, name),
        file_path: path.to_path_buf(),
        line_range: (start_line_with_comment(node), node.end_position().row + 1),
        signature,
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: Some(class_name.to_string()),
        visibility: Visibility::Public,
        parameters,
        return_type,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_ts_type_decl(
    node: Node,
    source: &[u8],
    path: &Path,
    decl_kind: &str,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();
    let visibility = detect_ts_export(node, source);

    let kind = match decl_kind {
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        _ => SymbolKind::TypeAlias,
    };

    Some(Symbol {
        kind,
        name: name.clone(),
        qualified_name: name,
        file_path: path.to_path_buf(),
        line_range: (start_line_with_comment(node), node.end_position().row + 1),
        signature: String::new(),
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: None,
        visibility,
        parameters: Vec::new(),
        return_type: None,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_ts_parameters(node: Node, source: &[u8]) -> Vec<Parameter> {
    let mut params = Vec::new();
    let params_node = node.child_by_field_name("parameters");
    let Some(params_node) = params_node else {
        return params;
    };
    for i in 0..params_node.child_count() {
        let Some(child) = params_node.child(i) else {
            continue;
        };
        match child.kind() {
            "required_parameter" | "optional_parameter" => {
                let name = child
                    .child_by_field_name("pattern")
                    .map(|n| node_text(n, source))
                    .unwrap_or_else(|| {
                        // Fallback: first identifier child
                        for j in 0..child.child_count() {
                            if let Some(c) = child.child(j) {
                                if c.kind() == "identifier" {
                                    return node_text(c, source);
                                }
                            }
                        }
                        String::new()
                    });
                let type_ann = child
                    .child_by_field_name("type")
                    .map(|n| node_text(n, source));
                params.push(Parameter {
                    name,
                    type_annotation: type_ann,
                });
            }
            "formal_parameters" => {
                // Nested — recurse
            }
            _ => {}
        }
    }
    params
}

/// Extract `export default <expr>` as a symbol named "default".
/// This handles patterns like `export default { ... } satisfies Type`
/// or `export default function() { ... }` that aren't otherwise captured.
fn extract_ts_export_default(node: Node, source: &[u8], path: &Path) -> Option<Symbol> {
    // Check if this export_statement has a "default" keyword
    let mut has_default = false;
    let mut has_declaration = false;
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "default" || node_text(child, source) == "default" {
                has_default = true;
            }
            // If it contains a named declaration, let the normal recursion handle it
            if matches!(
                child.kind(),
                "function_declaration"
                    | "class_declaration"
                    | "lexical_declaration"
                    | "interface_declaration"
                    | "type_alias_declaration"
                    | "enum_declaration"
            ) {
                has_declaration = true;
            }
        }
    }

    if !has_default || has_declaration {
        return None;
    }

    // This is `export default <expr>` — extract the whole thing as a symbol
    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    Some(Symbol {
        kind: SymbolKind::Constant,
        name: "default".to_string(),
        qualified_name: "default".to_string(),
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature: String::new(),
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: None,
        visibility: Visibility::Public,
        parameters: Vec::new(),
        return_type: None,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn detect_ts_export(node: Node, _source: &[u8]) -> Visibility {
    // Check if parent is export_statement
    if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            return Visibility::Public;
        }
    }
    Visibility::Private
}

// ============ Python ============

fn extract_python_node(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    let kind = node.kind();

    match kind {
        "function_definition" => {
            if let Some(sym) = extract_python_function(node, source, path, parent) {
                symbols.push(sym);
            }
            return;
        }
        "class_definition" => {
            if let Some(sym) = extract_python_class(node, source, path) {
                let name = sym.name.clone();
                symbols.push(sym);
                // Extract methods from class body
                if let Some(body) = node.child_by_field_name("body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            extract_python_node(child, source, path, Some(&name), symbols);
                        }
                    }
                }
                return;
            }
        }
        "decorated_definition" => {
            // Unwrap decorator to get the actual definition
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "function_definition" || child.kind() == "class_definition" {
                        extract_python_node(child, source, path, parent, symbols);
                    }
                }
            }
            return;
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_python_node(child, source, path, parent, symbols);
        }
    }
}

fn extract_python_function(
    node: Node,
    source: &[u8],
    path: &Path,
    parent: Option<&str>,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);

    let parameters = extract_python_parameters(node, source);
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| node_text(n, source));

    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();
    let signature = build_signature(&name, &parameters, &return_type);

    let visibility = if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    };

    let qualified_name = if let Some(p) = parent {
        format!("{}.{}", p, name)
    } else {
        name.clone()
    };

    let sym_kind = if parent.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    Some(Symbol {
        kind: sym_kind,
        name,
        qualified_name,
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature,
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: parent.map(String::from),
        visibility,
        parameters,
        return_type,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_python_class(node: Node, source: &[u8], path: &Path) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    let body_text = node_text(node, source);
    let normalized = normalize_body(&body_text);
    let body_hash = blake3::hash(normalized.as_bytes()).into();

    Some(Symbol {
        kind: SymbolKind::Class,
        name: name.clone(),
        qualified_name: name,
        file_path: path.to_path_buf(),
        line_range: (node.start_position().row + 1, node.end_position().row + 1),
        signature: String::new(),
        body_hash,
        body_text,
        normalized_body: normalized,
        parent: None,
        visibility: Visibility::Public,
        parameters: Vec::new(),
        return_type: None,
        ast_fingerprint: compute_ast_fingerprint(node, source),
    })
}

fn extract_python_parameters(node: Node, source: &[u8]) -> Vec<Parameter> {
    let mut params = Vec::new();
    let Some(params_node) = node.child_by_field_name("parameters") else {
        return params;
    };
    for i in 0..params_node.child_count() {
        let Some(child) = params_node.child(i) else {
            continue;
        };
        match child.kind() {
            "identifier" => {
                let name = node_text(child, source);
                if name != "self" && name != "cls" {
                    params.push(Parameter {
                        name,
                        type_annotation: None,
                    });
                }
            }
            "typed_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .or_else(|| child.child(0))
                    .map(|n| node_text(n, source))
                    .unwrap_or_default();
                if name != "self" && name != "cls" {
                    let type_ann = child
                        .child_by_field_name("type")
                        .map(|n| node_text(n, source));
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            "default_parameter" | "typed_default_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .or_else(|| child.child(0))
                    .map(|n| node_text(n, source))
                    .unwrap_or_default();
                if name != "self" && name != "cls" {
                    let type_ann = child
                        .child_by_field_name("type")
                        .map(|n| node_text(n, source));
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            _ => {}
        }
    }
    params
}

// ============ Helpers ============

/// Get the start line of a node, extended to include any preceding JSDoc/block comment.
/// Checks the previous sibling of the node (or its export_statement parent) for a comment.
fn start_line_with_comment(node: Node) -> usize {
    // The comment might be a sibling of the node itself, or of its export_statement parent
    let target = if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            parent
        } else {
            node
        }
    } else {
        node
    };

    if let Some(prev) = target.prev_sibling() {
        if prev.kind() == "comment" {
            // Include the comment in the line range
            return prev.start_position().row + 1;
        }
    }
    node.start_position().row + 1
}

fn node_text(node: Node, source: &[u8]) -> String {
    let start = node.start_byte();
    let end = node.end_byte();
    String::from_utf8_lossy(&source[start..end]).to_string()
}

fn build_signature(name: &str, params: &[Parameter], return_type: &Option<String>) -> String {
    let params_str = params
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(ret) = return_type {
        format!("{}({}) -> {}", name, params_str, ret)
    } else {
        format!("{}({})", name, params_str)
    }
}
