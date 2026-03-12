use ratatui::style::{Color, Modifier, Style};

use crate::diff::change::ChangeKind;

pub fn change_kind_style(kind: &ChangeKind) -> Style {
    match kind {
        ChangeKind::Added => Style::default().fg(Color::Green),
        ChangeKind::Deleted => Style::default().fg(Color::Red),
        ChangeKind::Renamed { .. } => Style::default().fg(Color::Yellow),
        ChangeKind::Moved { .. } => Style::default().fg(Color::Cyan),
        ChangeKind::MovedAndModified { .. } => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ChangeKind::Extracted { .. } => Style::default().fg(Color::Magenta),
        ChangeKind::Inlined { .. } => Style::default().fg(Color::Magenta),
        ChangeKind::SignatureChanged { .. } => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ChangeKind::BodyChanged => Style::default().fg(Color::Blue),
        ChangeKind::VisibilityChanged { .. } => Style::default().fg(Color::Gray),
    }
}

pub fn selected_style() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

pub fn header_style() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

pub fn diff_add_style() -> Style {
    Style::default().fg(Color::Green)
}

pub fn diff_del_style() -> Style {
    Style::default().fg(Color::Red)
}

pub fn diff_context_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn focus_border_style() -> Style {
    Style::default().fg(Color::Cyan)
}

pub fn normal_border_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Palette of distinct block colors shared by detail and summary panels
pub const BLOCK_COLORS: &[Color] = &[
    Color::Rgb(100, 140, 255), // Blue
    Color::Rgb(210, 130, 210), // Magenta
    Color::Rgb(80, 210, 200),  // Teal
    Color::Rgb(210, 190, 80),  // Amber
    Color::Rgb(80, 210, 120),  // Green
    Color::Rgb(210, 150, 80),  // Orange
    Color::Rgb(160, 120, 210), // Purple
    Color::Rgb(210, 100, 100), // Rose
];

pub fn risk_style(level: &crate::llm::review::RiskLevel) -> Style {
    match level {
        crate::llm::review::RiskLevel::Low => Style::default().fg(Color::Green),
        crate::llm::review::RiskLevel::Medium => Style::default().fg(Color::Yellow),
        crate::llm::review::RiskLevel::High => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}
