use serde::Serialize;
use std::path::PathBuf;

use crate::diff::change::{ChangeKind, SemanticChange};
use crate::llm::review::RiskLevel;
use crate::repo::call_graph::CallGraph;
use crate::repo::similarity::{SimilarCode, SimilarityKind};

#[derive(Debug, Clone, Serialize)]
pub struct AffectedCaller {
    pub caller_symbol: String,
    pub caller_file: PathBuf,
    pub caller_line: usize,
    pub changed_callee: String,
    pub change_description: String,
    pub risk: RiskLevel,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PatternWarning {
    pub message: String,
    pub changed_symbol: String,
    pub related_symbol: String,
    pub file_path: PathBuf,
    pub line_range: (usize, usize),
    pub similarity: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct ImpactAnalysis {
    pub affected_callers: Vec<AffectedCaller>,
    pub similar_code: Vec<SimilarCode>,
    pub pattern_warnings: Vec<PatternWarning>,
    pub risk_summary: ImpactRiskSummary,
}

#[derive(Debug, Default, Serialize)]
pub struct ImpactRiskSummary {
    pub high_risk_count: usize,
    pub medium_risk_count: usize,
    pub low_risk_count: usize,
    pub total_affected_callers: usize,
    pub total_similar_code: usize,
    pub total_pattern_warnings: usize,
}

/// Compute impact analysis from changes, call graph, and similarity results
pub fn analyze_impact(
    changes: &[SemanticChange],
    call_graph: &CallGraph,
    similar_code: &[SimilarCode],
    impact_depth: usize,
) -> ImpactAnalysis {
    let mut analysis = ImpactAnalysis::default();

    // Find affected callers for each change
    for change in changes {
        let symbol_name = change.symbol_name();
        let callers = call_graph.transitive_callers(symbol_name, impact_depth);

        let (change_desc, base_risk) = assess_change_risk(&change.kind);

        for (edge, depth) in &callers {
            let risk = if *depth == 0 {
                base_risk.clone()
            } else {
                // Lower risk for indirect callers
                match &base_risk {
                    RiskLevel::High => RiskLevel::Medium,
                    _ => RiskLevel::Low,
                }
            };

            analysis.affected_callers.push(AffectedCaller {
                caller_symbol: edge.caller.clone(),
                caller_file: edge.file_path.clone(),
                caller_line: edge.line,
                changed_callee: symbol_name.to_string(),
                change_description: change_desc.clone(),
                risk,
                depth: *depth,
            });
        }
    }

    // Deduplicate callers
    analysis.affected_callers.sort_by(|a, b| {
        a.caller_symbol
            .cmp(&b.caller_symbol)
            .then(a.caller_file.cmp(&b.caller_file))
    });
    analysis.affected_callers.dedup_by(|a, b| {
        a.caller_symbol == b.caller_symbol
            && a.caller_file == b.caller_file
            && a.changed_callee == b.changed_callee
    });

    // Add similar code
    analysis.similar_code = similar_code.to_vec();

    // Generate pattern warnings
    for sim in similar_code {
        match sim.kind {
            SimilarityKind::ExactDuplicate => {
                analysis.pattern_warnings.push(PatternWarning {
                    message: format!(
                        "Exact duplicate of '{}' exists but was not changed",
                        sim.changed_symbol
                    ),
                    changed_symbol: sim.changed_symbol.clone(),
                    related_symbol: sim.similar_symbol.clone(),
                    file_path: sim.file_path.clone(),
                    line_range: sim.line_range,
                    similarity: sim.similarity,
                });
            }
            SimilarityKind::StructurallySimilar if sim.similarity > 0.7 => {
                analysis.pattern_warnings.push(PatternWarning {
                    message: format!(
                        "'{}' is {:.0}% similar to changed '{}' - may need the same update",
                        sim.similar_symbol,
                        sim.similarity * 100.0,
                        sim.changed_symbol
                    ),
                    changed_symbol: sim.changed_symbol.clone(),
                    related_symbol: sim.similar_symbol.clone(),
                    file_path: sim.file_path.clone(),
                    line_range: sim.line_range,
                    similarity: sim.similarity,
                });
            }
            SimilarityKind::NamePattern if sim.similarity > 0.5 => {
                analysis.pattern_warnings.push(PatternWarning {
                    message: format!(
                        "'{}' follows the same naming pattern as '{}' - verify consistency",
                        sim.similar_symbol, sim.changed_symbol
                    ),
                    changed_symbol: sim.changed_symbol.clone(),
                    related_symbol: sim.similar_symbol.clone(),
                    file_path: sim.file_path.clone(),
                    line_range: sim.line_range,
                    similarity: sim.similarity,
                });
            }
            _ => {}
        }
    }

    // Compute risk summary
    analysis.risk_summary = compute_risk_summary(&analysis);

    analysis
}

fn assess_change_risk(kind: &ChangeKind) -> (String, RiskLevel) {
    match kind {
        ChangeKind::Deleted => ("symbol deleted".to_string(), RiskLevel::High),
        ChangeKind::SignatureChanged { .. } => {
            ("signature changed".to_string(), RiskLevel::High)
        }
        ChangeKind::Renamed { old_name, new_name } => (
            format!("renamed {} -> {}", old_name, new_name),
            RiskLevel::High,
        ),
        ChangeKind::Moved { from_file, to_file } => (
            format!("moved {} -> {}", from_file.display(), to_file.display()),
            RiskLevel::Medium,
        ),
        ChangeKind::MovedAndModified { from_file, to_file } => (
            format!(
                "moved and modified {} -> {}",
                from_file.display(),
                to_file.display()
            ),
            RiskLevel::High,
        ),
        ChangeKind::BodyChanged => ("body modified".to_string(), RiskLevel::Medium),
        ChangeKind::VisibilityChanged { .. } => {
            ("visibility changed".to_string(), RiskLevel::Medium)
        }
        ChangeKind::Extracted { .. } => ("code extracted".to_string(), RiskLevel::Medium),
        ChangeKind::Inlined { .. } => ("code inlined".to_string(), RiskLevel::Medium),
        ChangeKind::Added => ("new symbol".to_string(), RiskLevel::Low),
    }
}

fn compute_risk_summary(analysis: &ImpactAnalysis) -> ImpactRiskSummary {
    let mut summary = ImpactRiskSummary::default();
    summary.total_affected_callers = analysis.affected_callers.len();
    summary.total_similar_code = analysis.similar_code.len();
    summary.total_pattern_warnings = analysis.pattern_warnings.len();

    for caller in &analysis.affected_callers {
        match caller.risk {
            RiskLevel::High => summary.high_risk_count += 1,
            RiskLevel::Medium => summary.medium_risk_count += 1,
            RiskLevel::Low => summary.low_risk_count += 1,
        }
    }

    summary
}
