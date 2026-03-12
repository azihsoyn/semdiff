use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::tui::app::{App, PanelFocus};
use crate::tui::theme;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let border_style = if app.panel_focus == PanelFocus::Summary {
        theme::focus_border_style()
    } else {
        theme::normal_border_style()
    };

    let block = Block::default()
        .title(format!(" Changes ({}) ", app.change_count()))
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.diff_result.changes.is_empty() {
        let empty_msg = List::new(vec![
            ListItem::new(Line::raw("")),
            ListItem::new(Line::raw("  No semantic changes detected.")),
            ListItem::new(Line::raw("")),
            ListItem::new(Line::styled(
                "  Changed files may be in unsupported formats.",
                Style::default().fg(ratatui::style::Color::DarkGray),
            )),
            ListItem::new(Line::styled(
                "  Supported: .rs .go .ts .tsx .js .py .svelte",
                Style::default().fg(ratatui::style::Color::DarkGray),
            )),
        ])
        .block(block);
        f.render_widget(empty_msg, area);
        return;
    }

    // Build file-grouped items list
    // Each item maps to a change index or is a file header (mapped to first change in that file)
    let (items, index_map) = build_grouped_items(app);

    // Find the visual index for the selected change (skip None file headers)
    let visual_idx = index_map
        .iter()
        .position(|ci| *ci == Some(app.selected_index))
        .unwrap_or(0);
    app.summary_list_state.select(Some(visual_idx));

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_style())
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, &mut app.summary_list_state);
}

/// Build grouped list items and a map from visual index to change index
fn build_grouped_items(app: &App) -> (Vec<ListItem<'static>>, Vec<Option<usize>>) {
    let mut items: Vec<ListItem<'static>> = Vec::new();
    let mut index_map: Vec<Option<usize>> = Vec::new();

    // Group changes by file
    let mut file_groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut file_order: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (i, change) in app.diff_result.changes.iter().enumerate() {
        let file = change.file_info();
        if let Some(&idx) = file_order.get(&file) {
            file_groups[idx].1.push(i);
        } else {
            let idx = file_groups.len();
            file_order.insert(file.clone(), idx);
            file_groups.push((file, vec![i]));
        }
    }

    for (file, change_indices) in &file_groups {
        // File header
        let short_file = shorten_path(file);
        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!("📁 {} ({})", short_file, change_indices.len()),
            Style::default()
                .fg(ratatui::style::Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )])));
        // File headers are not selectable — use None
        index_map.push(None);

        // Change items under this file
        for &ci in change_indices {
            let change = &app.diff_result.changes[ci];
            let kind_style = theme::change_kind_style(&change.kind);
            let label = change.kind.label();
            let name = change.symbol_name();

            let confidence = if change.confidence < 1.0 {
                format!(" {:.0}%", change.confidence * 100.0)
            } else {
                String::new()
            };

            let line = Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("[{}]", label), kind_style),
                Span::raw(" "),
                Span::styled(
                    name.to_string(),
                    if ci == app.selected_index {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(
                    confidence,
                    Style::default().fg(ratatui::style::Color::DarkGray),
                ),
            ]);

            items.push(ListItem::new(line));
            index_map.push(Some(ci));
        }
    }

    (items, index_map)
}

/// Shorten a file path for display (show last 2-3 components)
fn shorten_path(path: &str) -> &str {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 3 {
        return path;
    }
    // Show last 3 path components
    let start = path.len()
        - parts[parts.len() - 3..]
            .iter()
            .map(|p| p.len() + 1)
            .sum::<usize>()
        + 1;
    &path[start..]
}
