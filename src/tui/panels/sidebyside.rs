use std::collections::HashSet;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::diff::body_diff::DiffLineTag;
use crate::diff::change::{ChangeKind, SemanticChange};
use crate::tui::app::App;

/// A colored block region in a file, with inline diff info
#[derive(Debug, Clone)]
struct BlockRegion {
    start_line: usize, // 1-based inclusive
    end_line: usize,   // 1-based inclusive
    color_idx: usize,  // index into BLOCK_COLORS palette
    #[allow(dead_code)]
    kind: BlockKind,
    is_selected: bool,
    /// Line numbers (1-based) within this region that are changed (added/deleted)
    changed_lines: HashSet<usize>,
    /// Label for the block (e.g. "~ modified", "fn foo ⟶ async-utils.ts")
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockKind {
    Moved,
    Added,
    Deleted,
    Modified,
    Extracted,
    Inlined,
}

/// Palette of distinct border/accent colors for blocks
const BLOCK_COLORS: &[Color] = &[
    Color::Rgb(100, 140, 255), // Blue
    Color::Rgb(210, 130, 210), // Magenta
    Color::Rgb(80, 210, 200),  // Teal
    Color::Rgb(210, 190, 80),  // Amber
    Color::Rgb(80, 210, 120),  // Green
    Color::Rgb(210, 150, 80),  // Orange
    Color::Rgb(160, 120, 210), // Purple
    Color::Rgb(210, 100, 100), // Rose
];

/// Render side-by-side view inline (called from detail panel)
pub fn render_inline(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(change) = app.selected_change() else {
        let p = Paragraph::new("No change selected");
        f.render_widget(p, area);
        return;
    };

    let old_file = change.old_symbol.as_ref().map(|s| s.file_path.clone());
    let new_file = change.new_symbol.as_ref().map(|s| s.file_path.clone());

    // File path header
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let header_halves = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(chunks[0]);

    let old_path_str = old_file
        .as_ref()
        .map(|p| format!(" ← {}", p.display()))
        .unwrap_or_else(|| " ← (none)".to_string());
    let new_path_str = new_file
        .as_ref()
        .map(|p| format!(" → {}", p.display()))
        .unwrap_or_else(|| " → (none)".to_string());

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            old_path_str,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))),
        header_halves[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            new_path_str,
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ))),
        header_halves[1],
    );

    let panels = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(chunks[1]);

    let all_changes: Vec<&SemanticChange> = app.diff_result.changes.iter().collect();
    let selected_idx = app.selected_index;
    let (old_regions, new_regions) = build_block_regions(
        &all_changes,
        old_file.as_deref(),
        new_file.as_deref(),
        selected_idx,
    );

    let old_content = old_file
        .as_ref()
        .and_then(|p| app.load_old_file_content(p))
        .map(|s| s.to_string());
    let new_content = new_file
        .as_ref()
        .and_then(|p| app.load_file_content(p))
        .map(|s| s.to_string());

    let old_fallback;
    let new_fallback;
    let old_display = if old_content.is_some() {
        old_content.as_deref()
    } else {
        old_fallback = app
            .selected_change()
            .and_then(|c| c.old_symbol.as_ref())
            .map(|s| s.body_text.clone());
        old_fallback.as_deref()
    };
    let new_display = if new_content.is_some() {
        new_content.as_deref()
    } else {
        new_fallback = app
            .selected_change()
            .and_then(|c| c.new_symbol.as_ref())
            .map(|s| s.body_text.clone());
        new_fallback.as_deref()
    };

    // scroll = file line number (0-based) to start from
    let scroll = app.detail_scroll;
    let panel_width = panels[0].width as usize;

    render_file_panel(f, panels[0], old_display, &old_regions, scroll, panel_width);
    render_file_panel(f, panels[1], new_display, &new_regions, scroll, panel_width);
}

// === Block region building ===

fn build_block_regions(
    changes: &[&SemanticChange],
    old_file: Option<&std::path::Path>,
    new_file: Option<&std::path::Path>,
    selected_idx: usize,
) -> (Vec<BlockRegion>, Vec<BlockRegion>) {
    let mut old_regions = Vec::new();
    let mut new_regions = Vec::new();
    let mut color_counter = 0usize;

    for (idx, change) in changes.iter().enumerate() {
        let kind = classify_block_kind(&change.kind);

        let touches_old = change
            .old_symbol
            .as_ref()
            .map_or(false, |s| old_file.map_or(false, |f| s.file_path == f));
        let touches_new = change
            .new_symbol
            .as_ref()
            .map_or(false, |s| new_file.map_or(false, |f| s.file_path == f));

        if !touches_old && !touches_new {
            continue;
        }

        let color_idx = color_counter % BLOCK_COLORS.len();
        let is_selected = idx == selected_idx;
        let (old_changed, new_changed) = compute_changed_lines(change);

        let old_label = build_label(change, kind, true);
        let new_label = build_label(change, kind, false);

        if let Some(ref sym) = change.old_symbol {
            if old_file.map_or(false, |f| sym.file_path == f) {
                old_regions.push(BlockRegion {
                    start_line: sym.line_range.0,
                    end_line: sym.line_range.1,
                    color_idx,
                    kind,
                    is_selected,
                    changed_lines: old_changed,
                    label: old_label,
                });
            }
        }

        if let Some(ref sym) = change.new_symbol {
            if new_file.map_or(false, |f| sym.file_path == f) {
                new_regions.push(BlockRegion {
                    start_line: sym.line_range.0,
                    end_line: sym.line_range.1,
                    color_idx,
                    kind,
                    is_selected,
                    changed_lines: new_changed,
                    label: new_label,
                });
            }
        }

        color_counter += 1;
    }

    old_regions.sort_by_key(|r| r.start_line);
    new_regions.sort_by_key(|r| r.start_line);
    (old_regions, new_regions)
}

fn build_label(change: &SemanticChange, kind: BlockKind, is_old: bool) -> String {
    let name = change.symbol_name();
    match kind {
        BlockKind::Moved => {
            if is_old {
                let dest = change
                    .new_symbol
                    .as_ref()
                    .map(|s| s.file_path.display().to_string())
                    .unwrap_or_default();
                format!("{} ⟶ {}", name, dest)
            } else {
                let src = change
                    .old_symbol
                    .as_ref()
                    .map(|s| s.file_path.display().to_string())
                    .unwrap_or_default();
                format!("{} ⟵ {}", name, src)
            }
        }
        BlockKind::Extracted => {
            if is_old {
                format!("{} ⟶ extracted", name)
            } else {
                format!("{} ⟵ extracted", name)
            }
        }
        BlockKind::Inlined => {
            if is_old {
                format!("{} ⟶ inlined", name)
            } else {
                format!("{} ⟵ inlined", name)
            }
        }
        BlockKind::Added => format!("+ {}", name),
        BlockKind::Deleted => format!("- {}", name),
        BlockKind::Modified => format!("~ {}", name),
    }
}

fn compute_changed_lines(change: &SemanticChange) -> (HashSet<usize>, HashSet<usize>) {
    let mut old_changed = HashSet::new();
    let mut new_changed = HashSet::new();

    if matches!(change.kind, ChangeKind::Added) {
        if let Some(ref sym) = change.new_symbol {
            for line in sym.line_range.0..=sym.line_range.1 {
                new_changed.insert(line);
            }
        }
        return (old_changed, new_changed);
    }

    if matches!(change.kind, ChangeKind::Deleted) {
        if let Some(ref sym) = change.old_symbol {
            for line in sym.line_range.0..=sym.line_range.1 {
                old_changed.insert(line);
            }
        }
        return (old_changed, new_changed);
    }

    let Some(ref diff) = change.body_diff else {
        return (old_changed, new_changed);
    };

    let old_start = change.old_symbol.as_ref().map(|s| s.line_range.0).unwrap_or(1);
    let new_start = change.new_symbol.as_ref().map(|s| s.line_range.0).unwrap_or(1);
    let mut old_line = old_start;
    let mut new_line = new_start;

    for dl in &diff.lines {
        match dl.tag {
            DiffLineTag::Equal => { old_line += 1; new_line += 1; }
            DiffLineTag::Delete => { old_changed.insert(old_line); old_line += 1; }
            DiffLineTag::Insert => { new_changed.insert(new_line); new_line += 1; }
        }
    }
    (old_changed, new_changed)
}

fn classify_block_kind(kind: &ChangeKind) -> BlockKind {
    match kind {
        ChangeKind::Added => BlockKind::Added,
        ChangeKind::Deleted => BlockKind::Deleted,
        ChangeKind::Moved { .. } | ChangeKind::MovedAndModified { .. } => BlockKind::Moved,
        ChangeKind::Extracted { .. } => BlockKind::Extracted,
        ChangeKind::Inlined { .. } => BlockKind::Inlined,
        _ => BlockKind::Modified,
    }
}

// === Rendering ===

/// Render one side of the side-by-side view with box-drawing borders around blocks.
/// `scroll` is a 0-based file line index to start from.
fn render_file_panel(
    f: &mut Frame,
    area: Rect,
    content: Option<&str>,
    regions: &[BlockRegion],
    scroll: usize,
    panel_width: usize,
) {
    let Some(content) = content else {
        let p = Paragraph::new(Span::styled(
            "(file not available)",
            Style::default().fg(Color::DarkGray),
        ));
        f.render_widget(p, area);
        return;
    };

    let file_lines: Vec<&str> = content.lines().collect();
    let visible_height = area.height as usize;
    let border_width = panel_width.saturating_sub(1); // width for ┌──┐ border lines

    // Build all virtual lines (file lines + border lines), then take [scroll_virt..scroll_virt+visible_height]
    let all_vlines = build_all_vlines(&file_lines, regions, border_width);

    // Find the virtual line index corresponding to the scroll file line
    let scroll_vline = file_line_to_vline(&all_vlines, scroll);

    let mut lines: Vec<Line> = Vec::with_capacity(visible_height);
    for i in scroll_vline..(scroll_vline + visible_height) {
        if i < all_vlines.len() {
            lines.push(all_vlines[i].clone());
        } else {
            lines.push(Line::from(Span::styled("~", Style::default().fg(Color::DarkGray))));
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Build all virtual lines for the file, interleaving border lines with content.
fn build_all_vlines<'a>(
    file_lines: &[&str],
    regions: &[BlockRegion],
    border_width: usize,
) -> Vec<Line<'a>> {
    let mut result: Vec<Line<'a>> = Vec::new();
    let mut region_idx = 0;
    let mut active_region: Option<&BlockRegion> = None;

    for file_idx in 0..file_lines.len() {
        let line_num = file_idx + 1; // 1-based

        // Check: should a new block start at this line?
        if active_region.is_none() {
            if region_idx < regions.len() && regions[region_idx].start_line == line_num {
                let r = &regions[region_idx];
                result.push(render_border_top(
                    BLOCK_COLORS[r.color_idx],
                    &r.label,
                    border_width,
                    r.is_selected,
                ));
                active_region = Some(r);
                region_idx += 1;
            }
        }

        // Render content line
        let region_info = active_region.map(|r| ContentRegionInfo {
            color: BLOCK_COLORS[r.color_idx],
            is_changed: r.changed_lines.contains(&line_num),
            is_selected: r.is_selected,
        });
        result.push(render_content_line(line_num, file_lines[file_idx], region_info));

        // Check: should the active block end at this line?
        if let Some(r) = active_region {
            if line_num >= r.end_line {
                result.push(render_border_bottom(
                    BLOCK_COLORS[r.color_idx],
                    border_width,
                    r.is_selected,
                ));
                active_region = None;
            }
        }
    }

    result
}

/// Find the virtual line index for a given 0-based file line scroll position.
/// Accounts for border lines inserted before/after blocks.
fn file_line_to_vline(all_vlines: &[Line], file_line_scroll: usize) -> usize {
    if file_line_scroll == 0 {
        return 0;
    }
    // Count how many content lines (not border lines) we've seen.
    // A content line has line number prefix; border lines start with ┌ or └.
    let mut content_count = 0usize;
    for (i, _line) in all_vlines.iter().enumerate() {
        // Heuristic: border lines start with box-drawing chars, content lines start with │ or space
        // Actually we can just count: every file line adds one Line, borders add extra.
        // So virtual index = file_line_scroll + number_of_border_lines_before_it.
        // Let's just walk and count.
        if is_content_line(all_vlines, i) {
            if content_count == file_line_scroll {
                // Scroll to 2 lines before this to show the border too
                return i.saturating_sub(1);
            }
            content_count += 1;
        }
    }
    all_vlines.len().saturating_sub(1)
}

/// Check if a virtual line at index i is a content line (not a border line).
/// Border lines are shorter or start with ┌/└.
fn is_content_line(all_vlines: &[Line], idx: usize) -> bool {
    if idx >= all_vlines.len() {
        return false;
    }
    let line = &all_vlines[idx];
    if line.spans.is_empty() {
        return false;
    }
    let first_char = line.spans[0].content.chars().next().unwrap_or(' ');
    first_char != '┌' && first_char != '└'
}

// === Line rendering ===

#[derive(Debug, Clone)]
struct ContentRegionInfo {
    color: Color,
    is_changed: bool,
    is_selected: bool,
}

fn render_border_top<'a>(color: Color, label: &str, width: usize, is_selected: bool) -> Line<'a> {
    let style = if is_selected {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };

    let prefix = "┌─ ";
    let suffix = " ─┐";
    let max_label = width.saturating_sub(prefix.len() + suffix.len() + 1);
    let label_trimmed: String = label.chars().take(max_label).collect();
    let fill_len = width.saturating_sub(
        prefix.chars().count() + label_trimmed.chars().count() + suffix.chars().count(),
    );
    let fill: String = "─".repeat(fill_len);

    Line::from(vec![Span::styled(
        format!("{}{}{}{}", prefix, label_trimmed, fill, suffix),
        style,
    )])
}

fn render_border_bottom<'a>(color: Color, width: usize, is_selected: bool) -> Line<'a> {
    let style = if is_selected {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };

    let fill_len = width.saturating_sub(2);
    let fill: String = "─".repeat(fill_len);
    Line::from(vec![Span::styled(format!("└{}┘", fill), style)])
}

fn render_content_line<'a>(
    line_num: usize,
    text: &str,
    region_info: Option<ContentRegionInfo>,
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();

    if let Some(info) = region_info {
        let border_style = if info.is_selected {
            Style::default().fg(info.color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(info.color)
        };

        let num_style = if info.is_changed {
            Style::default().fg(info.color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(darken_color(info.color, 50))
        };

        let text_style = if info.is_changed {
            Style::default()
                .fg(Color::White)
                .bg(color_to_bg(info.color))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(180, 180, 180))
        };

        spans.push(Span::styled("│", border_style));
        spans.push(Span::styled(format!("{:4} ", line_num), num_style));
        spans.push(Span::styled(text.to_string(), text_style));
    } else {
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(
            format!("{:4} ", line_num),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(text.to_string(), Style::default().fg(Color::Gray)));
    }

    Line::from(spans)
}

fn color_to_bg(color: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(r / 4, g / 4, b / 4),
        other => other,
    }
}

fn darken_color(color: Color, amount: u8) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            r.saturating_sub(amount),
            g.saturating_sub(amount),
            b.saturating_sub(amount),
        ),
        other => other,
    }
}
