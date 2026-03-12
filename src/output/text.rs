use crate::diff::change::{ChangeKind, DiffResult, SemanticChange};
use crate::repo::RepoAnalysis;

pub fn print_diff(result: &DiffResult) {
    println!("=== Semantic Diff Summary ===");
    println!();
    println!(
        "Total changes: {}  (Added: {}, Deleted: {}, Renamed: {}, Moved: {}, Extracted: {}, Modified: {})",
        result.summary.total_changes,
        result.summary.added,
        result.summary.deleted,
        result.summary.renamed,
        result.summary.moved,
        result.summary.extracted,
        result.summary.modified,
    );
    println!();

    if result.changes.is_empty() {
        println!("No semantic changes detected.");
        return;
    }

    let categories: Vec<(&str, Box<dyn Fn(&ChangeKind) -> bool>)> = vec![
        ("Moved", Box::new(|c| matches!(c, ChangeKind::Moved { .. } | ChangeKind::MovedAndModified { .. }))),
        ("Extracted", Box::new(|c| matches!(c, ChangeKind::Extracted { .. }))),
        ("Inlined", Box::new(|c| matches!(c, ChangeKind::Inlined { .. }))),
        ("Renamed", Box::new(|c| matches!(c, ChangeKind::Renamed { .. }))),
        ("Signature Changed", Box::new(|c| matches!(c, ChangeKind::SignatureChanged { .. }))),
        ("Body Modified", Box::new(|c| matches!(c, ChangeKind::BodyChanged))),
        ("Visibility Changed", Box::new(|c| matches!(c, ChangeKind::VisibilityChanged { .. }))),
        ("Added", Box::new(|c| matches!(c, ChangeKind::Added))),
        ("Deleted", Box::new(|c| matches!(c, ChangeKind::Deleted))),
    ];

    for (label, filter) in &categories {
        let items: Vec<&SemanticChange> = result.changes.iter().filter(|c| filter(&c.kind)).collect();
        if items.is_empty() {
            continue;
        }

        println!("--- {} ({}) ---", label, items.len());
        for change in &items {
            let name = change.symbol_name();
            let desc = change.kind.short_description();
            let file = change.file_info();
            let confidence = if change.confidence < 1.0 {
                format!(" ({:.0}%)", change.confidence * 100.0)
            } else {
                String::new()
            };

            println!("  [{}] {}{}", change.kind.label(), name, confidence);
            if !desc.is_empty() {
                println!("       {}", desc);
            }
            if !file.is_empty() {
                println!("       @ {}", file);
            }

            if let Some(ref diff) = change.body_diff {
                let plain = diff.to_plain_text();
                let lines: Vec<&str> = plain.lines().take(10).collect();
                for line in &lines {
                    println!("       {}", line);
                }
                let total_lines = plain.lines().count();
                if total_lines > 10 {
                    println!("       ... ({} more lines)", total_lines - 10);
                }
            }
            println!();
        }
    }
}

pub fn print_repo_analysis(analysis: &RepoAnalysis) {
    println!();
    println!("=== Repo-Wide Impact Analysis ===");
    println!(
        "Scanned: {} symbols, {} call edges",
        analysis.total_repo_symbols, analysis.call_graph_edges
    );
    println!();

    let impact = &analysis.impact;

    // Risk summary
    let rs = &impact.risk_summary;
    if rs.total_affected_callers > 0 || rs.total_pattern_warnings > 0 {
        println!(
            "Risk: {} high, {} medium, {} low",
            rs.high_risk_count, rs.medium_risk_count, rs.low_risk_count
        );
        println!();
    }

    // Affected callers
    if !impact.affected_callers.is_empty() {
        println!("--- Affected Callers ({}) ---", impact.affected_callers.len());
        for caller in &impact.affected_callers {
            let depth_indicator = if caller.depth > 0 {
                format!(" (depth {})", caller.depth)
            } else {
                String::new()
            };
            println!(
                "  [{}] {} @ {}:{}{}",
                match caller.risk {
                    crate::llm::review::RiskLevel::High => "HIGH",
                    crate::llm::review::RiskLevel::Medium => "MED ",
                    crate::llm::review::RiskLevel::Low => "LOW ",
                },
                caller.caller_symbol,
                caller.caller_file.display(),
                caller.caller_line,
                depth_indicator,
            );
            println!(
                "       calls '{}' which was: {}",
                caller.changed_callee, caller.change_description
            );
        }
        println!();
    }

    // Similar code
    if !impact.similar_code.is_empty() {
        println!("--- Similar Code ({}) ---", impact.similar_code.len());
        for sim in &impact.similar_code {
            println!(
                "  [{}] {} @ {}:L{}-{}  ({:.0}% similar to {})",
                sim.kind.label(),
                sim.similar_symbol,
                sim.file_path.display(),
                sim.line_range.0,
                sim.line_range.1,
                sim.similarity * 100.0,
                sim.changed_symbol,
            );
        }
        println!();
    }

    // Pattern warnings
    if !impact.pattern_warnings.is_empty() {
        println!("--- Pattern Warnings ({}) ---", impact.pattern_warnings.len());
        for warning in &impact.pattern_warnings {
            println!(
                "  [WARN] {}",
                warning.message,
            );
            println!(
                "         @ {}:L{}-{}",
                warning.file_path.display(),
                warning.line_range.0,
                warning.line_range.1,
            );
        }
        println!();
    }

    if impact.affected_callers.is_empty()
        && impact.similar_code.is_empty()
        && impact.pattern_warnings.is_empty()
    {
        println!("No impact detected.");
    }
}
