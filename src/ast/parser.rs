use anyhow::{Context, Result};
use std::path::Path;
use tree_sitter::{Parser, Tree};

use super::language::Language;

pub fn parse_file(source: &[u8], language: Language) -> Result<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.tree_sitter_language())
        .context("Failed to set tree-sitter language")?;

    parser
        .parse(source, None)
        .context("Failed to parse source code")
}

/// Returns (tree, language, effective_source) — effective_source is Some if
/// the source was transformed (e.g., script extraction from .svelte).
pub fn parse_file_auto(source: &[u8], path: &Path) -> Result<(Tree, Language, Option<Vec<u8>>)> {
    let language = Language::detect(path)
        .with_context(|| format!("Unsupported file type: {}", path.display()))?;

    // For .svelte files, extract <script> block content
    let effective_source = if path.extension().and_then(|e| e.to_str()) == Some("svelte") {
        extract_svelte_script(source)
    } else {
        None
    };

    let src = effective_source.as_deref().unwrap_or(source);
    let tree = parse_file(src, language)?;
    Ok((tree, language, effective_source))
}

/// Extract the content of <script> or <script lang="ts"> blocks from a .svelte file
fn extract_svelte_script(source: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(source).ok()?;
    let mut scripts = Vec::new();

    // Find all <script...>...</script> blocks
    let mut search_from = 0;
    while let Some(start_tag_begin) = text[search_from..].find("<script") {
        let start_tag_begin = search_from + start_tag_begin;
        let start_tag_end = text[start_tag_begin..].find('>')? + start_tag_begin + 1;
        let end_tag = text[start_tag_end..].find("</script>")? + start_tag_end;

        let script_content = &text[start_tag_end..end_tag];
        scripts.push(script_content);
        search_from = end_tag + "</script>".len();
    }

    if scripts.is_empty() {
        return None;
    }

    Some(scripts.join("\n").into_bytes())
}
