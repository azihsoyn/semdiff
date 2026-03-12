pub mod call_refs;
pub mod language;
pub mod parser;
pub mod query;
pub mod symbol;

use anyhow::Result;
use std::path::Path;

use call_refs::CallReference;
use language::Language;
use symbol::Symbol;

/// Parse a file and extract all symbols
pub fn extract_file_symbols(path: &Path) -> Result<Vec<Symbol>> {
    let source = std::fs::read(path)?;
    let (tree, lang, effective) = parser::parse_file_auto(&source, path)?;
    let src = effective.as_deref().unwrap_or(&source);
    Ok(query::extract_symbols(&tree, src, path, lang))
}

/// Parse in-memory bytes and extract symbols (for git mode)
pub fn extract_symbols_from_bytes(source: &[u8], path: &Path) -> Result<Vec<Symbol>> {
    let (tree, lang, effective) = parser::parse_file_auto(source, path)?;
    let src = effective.as_deref().unwrap_or(source);
    Ok(query::extract_symbols(&tree, src, path, lang))
}

/// Parse in-memory bytes and extract call references
pub fn extract_calls_from_bytes(source: &[u8], path: &Path) -> Result<Vec<CallReference>> {
    let (tree, lang, effective) = parser::parse_file_auto(source, path)?;
    let src = effective.as_deref().unwrap_or(source);
    Ok(call_refs::extract_call_references(&tree, src, path, lang))
}

/// Check if a file is supported
pub fn is_supported(path: &Path) -> bool {
    Language::detect(path).is_some()
}
