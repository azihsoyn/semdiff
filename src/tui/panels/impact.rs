use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::repo::RepoAnalysis;
use crate::tui::app::{App, PanelFocus};
use crate::tui::theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let border_style = if app.panel_focus == PanelFocus::Impact {
        theme::focus_border_style()
    } else {
        theme::normal_border_style()
    };

    let Some(ref analysis) = app.repo_analysis else {
        let block = Block::default()
            .title(" Impact ")
            .borders(Borders::ALL)
            .border_style(border_style);
        let empty = Paragraph::new("No repo analysis available. Use --repo-analysis flag.")
            .block(block)
            .wrap(Wrap { trim: false });
        f.render_widget(empty, area);
        return;
    };

    let block = Block::default()
        .title(format!(
            " Impact ({} callers, {} similar, {} warnings) ",
            analysis.impact.affected_callers.len(),
            analysis.impact.similar_code.len(),
            analysis.impact.pattern_warnings.len(),
        ))
        .borders(Borders::ALL)
        .border_style(border_style);

    let lines = build_impact_lines(analysis, app);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((app.bottom_scroll as u16, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn build_impact_lines(analysis: &RepoAnalysis, app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let selected_name = app
        .selected_change()
        .map(|c| c.symbol_name().to_string())
        .unwrap_or_default();

    // Filter to show impact relevant to the selected change
    let relevant_callers: Vec<_> = analysis
        .impact
        .affected_callers
        .iter()
        .filter(|c| c.changed_callee == selected_name)
        .collect();

    let relevant_similar: Vec<_> = analysis
        .impact
        .similar_code
        .iter()
        .filter(|s| s.changed_symbol == selected_name)
        .collect();

    let relevant_warnings: Vec<_> = analysis
        .impact
        .pattern_warnings
        .iter()
        .filter(|w| w.changed_symbol == selected_name)
        .collect();

    if relevant_callers.is_empty() && relevant_similar.is_empty() && relevant_warnings.is_empty() {
        // Show overall summary instead
        lines.push(Line::from(Span::styled(
            "No specific impact for selected change. Overall:",
            Style::default().fg(ratatui::style::Color::DarkGray),
        )));
        lines.push(Line::from(""));

        let rs = &analysis.impact.risk_summary;
        lines.push(Line::from(format!(
            "Total: {} affected callers, {} similar code, {} warnings",
            rs.total_affected_callers, rs.total_similar_code, rs.total_pattern_warnings
        )));

        // Show top warnings
        for warning in analysis.impact.pattern_warnings.iter().take(5) {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("[WARN] ", Style::default().fg(ratatui::style::Color::Yellow)),
                Span::raw(warning.message.clone()),
            ]));
            lines.push(Line::from(format!(
                "       @ {}:L{}-{}",
                warning.file_path.display(),
                warning.line_range.0,
                warning.line_range.1
            )));
        }

        return lines;
    }

    // Affected callers
    if !relevant_callers.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Affected Callers ({})", relevant_callers.len()),
            theme::header_style(),
        )));
        for caller in &relevant_callers {
            let risk_style = match caller.risk {
                crate::llm::review::RiskLevel::High => {
                    Style::default().fg(ratatui::style::Color::Red)
                }
                crate::llm::review::RiskLevel::Medium => {
                    Style::default().fg(ratatui::style::Color::Yellow)
                }
                crate::llm::review::RiskLevel::Low => {
                    Style::default().fg(ratatui::style::Color::Green)
                }
            };
            let depth = if caller.depth > 0 {
                format!(" (depth {})", caller.depth)
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "  [{}]",
                        match caller.risk {
                            crate::llm::review::RiskLevel::High => "HIGH",
                            crate::llm::review::RiskLevel::Medium => "MED ",
                            crate::llm::review::RiskLevel::Low => "LOW ",
                        }
                    ),
                    risk_style,
                ),
                Span::raw(format!(
                    " {} @ {}:{}{}",
                    caller.caller_symbol,
                    caller.caller_file.display(),
                    caller.caller_line,
                    depth,
                )),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Similar code
    if !relevant_similar.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Similar Code ({})", relevant_similar.len()),
            theme::header_style(),
        )));
        for sim in &relevant_similar {
            lines.push(Line::from(format!(
                "  [{}] {} @ {}:L{}-{}  ({:.0}%)",
                sim.kind.label(),
                sim.similar_symbol,
                sim.file_path.display(),
                sim.line_range.0,
                sim.line_range.1,
                sim.similarity * 100.0,
            )));
        }
        lines.push(Line::from(""));
    }

    // Warnings
    if !relevant_warnings.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Warnings ({})", relevant_warnings.len()),
            theme::header_style(),
        )));
        for warning in &relevant_warnings {
            lines.push(Line::from(vec![
                Span::styled("[WARN] ", Style::default().fg(ratatui::style::Color::Yellow)),
                Span::raw(warning.message.clone()),
            ]));
        }
    }

    lines
}
