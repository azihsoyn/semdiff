use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::llm::review::ReviewResult;
use crate::tui::app::{App, PanelFocus};
use crate::tui::theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let border_style = if app.panel_focus == PanelFocus::Review {
        theme::focus_border_style()
    } else {
        theme::normal_border_style()
    };

    let block = Block::default()
        .title(" Review ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.loading_review {
        let loading = Paragraph::new("Loading review from LLM...")
            .block(block)
            .wrap(Wrap { trim: false });
        f.render_widget(loading, area);
        return;
    }

    // Show per-change review if available, otherwise overall
    let review = app
        .current_review()
        .or(app.overall_review.as_ref());

    let Some(review) = review else {
        let hint = if app.llm_enabled {
            "Press 'r' to request LLM review for this change, 'R' for overall review"
        } else {
            "LLM review not enabled. Use --llm-review flag with an API key"
        };
        let empty = Paragraph::new(hint)
            .block(block)
            .wrap(Wrap { trim: false });
        f.render_widget(empty, area);
        return;
    };

    let lines = build_review_lines(review);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((app.bottom_scroll as u16, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn build_review_lines(review: &ReviewResult) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Risk level
    let risk_style = theme::risk_style(&review.risk_level);
    lines.push(Line::from(vec![
        Span::styled("Risk: ", Style::default()),
        Span::styled(review.risk_level.to_string(), risk_style),
    ]));
    lines.push(Line::from(""));

    // Summary
    lines.push(Line::from(Span::styled(
        "Summary",
        theme::header_style(),
    )));
    for line in review.summary.lines() {
        lines.push(Line::from(line.to_string()));
    }
    lines.push(Line::from(""));

    // Key observations
    if !review.key_observations.is_empty() {
        lines.push(Line::from(Span::styled(
            "Key Observations",
            theme::header_style(),
        )));
        for obs in &review.key_observations {
            lines.push(Line::from(format!("  • {}", obs)));
        }
        lines.push(Line::from(""));
    }

    // Potential issues
    if !review.potential_issues.is_empty() {
        lines.push(Line::from(Span::styled(
            "Potential Issues",
            theme::header_style(),
        )));
        for issue in &review.potential_issues {
            let sev_style = theme::risk_style(&issue.severity);
            lines.push(Line::from(vec![
                Span::styled(format!("  [{}] ", issue.severity), sev_style),
                Span::raw(issue.description.clone()),
            ]));
            if let Some(ref suggestion) = issue.suggestion {
                lines.push(Line::from(format!("    → {}", suggestion)));
            }
        }
        lines.push(Line::from(""));
    }

    // Test suggestions
    if !review.test_suggestions.is_empty() {
        lines.push(Line::from(Span::styled(
            "Test Suggestions",
            theme::header_style(),
        )));
        for test in &review.test_suggestions {
            lines.push(Line::from(format!("  • {}", test)));
        }
    }

    lines
}
