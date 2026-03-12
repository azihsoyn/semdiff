use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::diff::change::SemanticChange;
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

    // Sort file groups by directory path (like a file explorer tree)
    file_groups.sort_by(|(a, _), (b, _)| cmp_file_paths(a, b));

    // Sort changes within each file by line number (top of file first)
    for (_, indices) in &mut file_groups {
        indices.sort_by_key(|&i| {
            let change = &app.diff_result.changes[i];
            change
                .new_symbol
                .as_ref()
                .or(change.old_symbol.as_ref())
                .map(|s| s.line_range.0)
                .unwrap_or(0)
        });
    }

    for (file, change_indices) in &file_groups {
        let is_collapsed = app.collapsed_files.contains(file);
        let arrow = if is_collapsed { "▸" } else { "▾" };

        // File header
        let short_file = shorten_path(file);
        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!("{} [{}] ({})", arrow, short_file, change_indices.len()),
            Style::default()
                .fg(ratatui::style::Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )])));
        // Map file header to the first change in the group (so it's selectable for collapse toggle)
        index_map.push(Some(change_indices[0]));

        if is_collapsed {
            continue;
        }

        // Compute nesting depth for indentation
        let nesting = compute_nesting(change_indices, &app.diff_result.changes);

        // Change items under this file (with block colors matching detail panel)
        for (color_counter, &ci) in change_indices.iter().enumerate() {
            let change = &app.diff_result.changes[ci];
            let kind_style = theme::change_kind_style(&change.kind);
            let label = change.kind.label();
            let name = change.symbol_name();
            let block_color = theme::BLOCK_COLORS[color_counter % theme::BLOCK_COLORS.len()];

            let confidence = if change.confidence < 1.0 {
                format!(" {:.0}%", change.confidence * 100.0)
            } else {
                String::new()
            };

            let name_style = if ci == app.selected_index {
                Style::default()
                    .fg(block_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(block_color)
            };

            let depth = nesting[color_counter];
            let indent = "  ".repeat(depth + 1);

            let line = Line::from(vec![
                Span::raw(indent),
                Span::styled(format!("[{}]", label), kind_style),
                Span::raw(" "),
                Span::styled(name.to_string(), name_style),
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

/// Compare file paths for directory-structure ordering.
/// Files in the same directory group together; directories sort before deeper paths.
/// Within the same directory, files are sorted alphabetically.
fn cmp_file_paths(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts: Vec<&str> = a.split('/').collect();
    let b_parts: Vec<&str> = b.split('/').collect();

    // Compare path components one by one
    for (ap, bp) in a_parts.iter().zip(b_parts.iter()) {
        let ac = ap.to_lowercase();
        let bc = bp.to_lowercase();
        match ac.cmp(&bc) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    // Shorter path (shallower) comes first
    a_parts.len().cmp(&b_parts.len())
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

/// Compute nesting depth for each change within a file group.
/// A symbol is nested if its line range is contained within a preceding symbol's range.
fn compute_nesting(indices: &[usize], changes: &[SemanticChange]) -> Vec<usize> {
    let mut depths = vec![0usize; indices.len()];
    // Stack of (end_line, depth)
    let mut stack: Vec<(usize, usize)> = Vec::new();

    for (pos, &ci) in indices.iter().enumerate() {
        let change = &changes[ci];
        let sym = change.new_symbol.as_ref().or(change.old_symbol.as_ref());
        if let Some(sym) = sym {
            // Pop stack entries whose ranges have ended before this symbol starts
            while let Some(&(end, _)) = stack.last() {
                if sym.line_range.0 > end {
                    stack.pop();
                } else {
                    break;
                }
            }

            let depth = stack.last().map_or(0, |&(_, d)| d + 1);
            depths[pos] = depth;

            // Push this symbol's range onto the stack
            stack.push((sym.line_range.1, depth));
        }
    }

    depths
}
