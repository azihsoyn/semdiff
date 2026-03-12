use crate::diff::change::{DiffResult, SemanticChange};

/// Build a review prompt for a single change
pub fn build_change_review_prompt(change: &SemanticChange) -> String {
    let mut prompt = String::new();

    prompt.push_str("You are a code reviewer. Analyze the following semantic change and provide a review.\n\n");

    prompt.push_str(&format!("## Change Type: {}\n", change.kind.label()));
    prompt.push_str(&format!("## Description: {}\n", change.kind.short_description()));
    prompt.push_str(&format!("## Confidence: {:.0}%\n\n", change.confidence * 100.0));

    if let Some(ref old_sym) = change.old_symbol {
        prompt.push_str(&format!("### Old Symbol\n"));
        prompt.push_str(&format!("- Name: {}\n", old_sym.qualified_name));
        prompt.push_str(&format!("- File: {}:{}-{}\n", old_sym.file_path.display(), old_sym.line_range.0, old_sym.line_range.1));
        prompt.push_str(&format!("- Kind: {}\n", old_sym.kind));
        if !old_sym.signature.is_empty() {
            prompt.push_str(&format!("- Signature: {}\n", old_sym.signature));
        }
        prompt.push('\n');
    }

    if let Some(ref new_sym) = change.new_symbol {
        prompt.push_str(&format!("### New Symbol\n"));
        prompt.push_str(&format!("- Name: {}\n", new_sym.qualified_name));
        prompt.push_str(&format!("- File: {}:{}-{}\n", new_sym.file_path.display(), new_sym.line_range.0, new_sym.line_range.1));
        prompt.push_str(&format!("- Kind: {}\n", new_sym.kind));
        if !new_sym.signature.is_empty() {
            prompt.push_str(&format!("- Signature: {}\n", new_sym.signature));
        }
        prompt.push('\n');
    }

    if let Some(ref diff) = change.body_diff {
        let plain = diff.to_plain_text();
        prompt.push_str("### Body Diff\n```\n");
        // Truncate if too long
        if plain.len() > 3000 {
            prompt.push_str(&plain[..3000]);
            prompt.push_str("\n... (truncated)\n");
        } else {
            prompt.push_str(&plain);
        }
        prompt.push_str("```\n\n");
    }

    prompt.push_str("Please respond in JSON with the following structure:\n");
    prompt.push_str(r#"{
  "summary": "Brief description of what this change does",
  "risk_level": "Low" | "Medium" | "High",
  "key_observations": ["observation 1", "observation 2"],
  "potential_issues": [{"severity": "Low"|"Medium"|"High", "description": "...", "suggestion": "..."}],
  "test_suggestions": ["test case 1", "test case 2"]
}
"#);

    prompt
}

/// Build a summary prompt for the entire diff result
pub fn build_summary_prompt(result: &DiffResult) -> String {
    let mut prompt = String::new();

    prompt.push_str("You are a code reviewer. Provide a high-level summary of the following semantic diff.\n\n");

    prompt.push_str(&format!("## Diff Summary\n"));
    prompt.push_str(&format!("- Total changes: {}\n", result.summary.total_changes));
    prompt.push_str(&format!("- Added: {}\n", result.summary.added));
    prompt.push_str(&format!("- Deleted: {}\n", result.summary.deleted));
    prompt.push_str(&format!("- Renamed: {}\n", result.summary.renamed));
    prompt.push_str(&format!("- Moved: {}\n", result.summary.moved));
    prompt.push_str(&format!("- Extracted: {}\n", result.summary.extracted));
    prompt.push_str(&format!("- Inlined: {}\n", result.summary.inlined));
    prompt.push_str(&format!("- Modified: {}\n", result.summary.modified));
    prompt.push_str(&format!("- Signature changed: {}\n\n", result.summary.signature_changed));

    prompt.push_str("## Changes\n");
    for (i, change) in result.changes.iter().enumerate() {
        if i >= 50 {
            prompt.push_str(&format!("... and {} more changes\n", result.changes.len() - 50));
            break;
        }
        let name = change.symbol_name();
        let kind = change.kind.label();
        let desc = change.kind.short_description();
        let file = change.file_info();
        prompt.push_str(&format!("- [{}] {} - {} ({})\n", kind, name, desc, file));
    }

    prompt.push_str("\nPlease respond in JSON with the following structure:\n");
    prompt.push_str(r#"{
  "summary": "High-level summary of all changes",
  "risk_level": "Low" | "Medium" | "High",
  "key_observations": ["observation 1", "observation 2"],
  "potential_issues": [{"severity": "Low"|"Medium"|"High", "description": "...", "suggestion": "..."}],
  "test_suggestions": ["test case 1", "test case 2"]
}
"#);

    prompt
}
