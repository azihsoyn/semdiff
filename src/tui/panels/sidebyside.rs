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
    color_idx: usize,
    is_selected: bool,
    changed_lines: HashSet<usize>,
    label: String,
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

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let header_halves =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[0]);

    let old_path_str = old_file
        .as_ref()
        .map(|p| format!(" <- {}", p.display()))
        .unwrap_or_else(|| " <- (none)".to_string());
    let new_path_str = new_file
        .as_ref()
        .map(|p| format!(" -> {}", p.display()))
        .unwrap_or_else(|| " -> (none)".to_string());

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
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))),
        header_halves[1],
    );

    let panels =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

    // Load file content (requires &mut app) before borrowing changes
    let old_content = old_file
        .as_ref()
        .and_then(|p| app.load_old_file_content(p))
        .map(|s| s.to_string());
    let new_content = new_file
        .as_ref()
        .and_then(|p| app.load_file_content(p))
        .map(|s| s.to_string());

    let old_fallback = if old_content.is_none() {
        app.selected_change()
            .and_then(|c| c.old_symbol.as_ref())
            .map(|s| s.body_text.clone())
    } else {
        None
    };
    let new_fallback = if new_content.is_none() {
        app.selected_change()
            .and_then(|c| c.new_symbol.as_ref())
            .map(|s| s.body_text.clone())
    } else {
        None
    };

    let old_display = old_content.as_deref().or(old_fallback.as_deref());
    let new_display = new_content.as_deref().or(new_fallback.as_deref());

    let all_changes: Vec<&SemanticChange> = app.diff_result.changes.iter().collect();
    let selected_idx = app.selected_index;
    let (old_regions, new_regions) = build_block_regions(
        &all_changes,
        old_file.as_deref(),
        new_file.as_deref(),
        selected_idx,
    );

    let scroll = app.detail_scroll;
    let h_scroll = app.detail_h_scroll;
    let panel_width = panels[0].width as usize;
    let visible_height = panels[0].height as usize;
    let border_width = panel_width.saturating_sub(1);

    let old_text = old_display.unwrap_or("");
    let new_text = new_display.unwrap_or("");

    // Phase 1: Build aligned rows using file-level diff
    let aligned_rows = build_aligned_rows(old_text, new_text);

    // Phase 2: Render rows with block overlays and insert borders
    let (old_vlines, new_vlines) = render_aligned_rows(
        &aligned_rows,
        old_text,
        new_text,
        &old_regions,
        &new_regions,
        border_width,
        h_scroll,
    );

    // Phase 3: Scroll and display
    let scroll_vline = find_scroll_vline(&old_vlines, scroll);

    let mut old_visible: Vec<Line> = Vec::with_capacity(visible_height);
    let mut new_visible: Vec<Line> = Vec::with_capacity(visible_height);

    for i in scroll_vline..(scroll_vline + visible_height) {
        old_visible.push(
            old_vlines
                .get(i)
                .cloned()
                .unwrap_or_else(tilde_line),
        );
        new_visible.push(
            new_vlines
                .get(i)
                .cloned()
                .unwrap_or_else(tilde_line),
        );
    }

    f.render_widget(Paragraph::new(old_visible), panels[0]);
    f.render_widget(Paragraph::new(new_visible), panels[1]);
}

fn tilde_line<'a>() -> Line<'a> {
    Line::from(Span::styled("~", Style::default().fg(Color::DarkGray)))
}

// === Phase 1: Build aligned rows from file-level diff ===

/// An aligned row: what to show on old side and new side
#[derive(Debug, Clone)]
enum AlignedRow {
    /// Both sides show a line (equal in file diff)
    Both { old_line: usize, new_line: usize },
    /// Only old side (deleted line)
    OldOnly { old_line: usize },
    /// Only new side (inserted line)
    NewOnly { new_line: usize },
}

fn build_aligned_rows(old_text: &str, new_text: &str) -> Vec<AlignedRow> {
    use similar::{ChangeTag, TextDiff};

    let file_diff = TextDiff::from_lines(old_text, new_text);
    let mut rows = Vec::new();
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;

    for change in file_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                old_line += 1;
                new_line += 1;
                rows.push(AlignedRow::Both { old_line, new_line });
            }
            ChangeTag::Delete => {
                old_line += 1;
                rows.push(AlignedRow::OldOnly { old_line });
            }
            ChangeTag::Insert => {
                new_line += 1;
                rows.push(AlignedRow::NewOnly { new_line });
            }
        }
    }
    rows
}

// === Phase 2: Render aligned rows with block borders ===

fn render_aligned_rows<'a>(
    rows: &[AlignedRow],
    old_text: &str,
    new_text: &str,
    old_regions: &[BlockRegion],
    new_regions: &[BlockRegion],
    border_width: usize,
    h_scroll: usize,
) -> (Vec<Line<'a>>, Vec<Line<'a>>) {
    let old_file_lines: Vec<&str> = old_text.lines().collect();
    let new_file_lines: Vec<&str> = new_text.lines().collect();

    let old_region_map = build_region_map(old_regions);
    let new_region_map = build_region_map(new_regions);

    let mut old_result: Vec<Line<'a>> = Vec::new();
    let mut new_result: Vec<Line<'a>> = Vec::new();

    // Track which blocks have had their top/bottom border emitted
    let mut old_top_emitted: HashSet<usize> = HashSet::new();
    let mut old_bottom_emitted: HashSet<usize> = HashSet::new();
    let mut new_top_emitted: HashSet<usize> = HashSet::new();
    let mut new_bottom_emitted: HashSet<usize> = HashSet::new();

    for row in rows {
        let (old_ln, new_ln) = match row {
            AlignedRow::Both { old_line, new_line } => (Some(*old_line), Some(*new_line)),
            AlignedRow::OldOnly { old_line } => (Some(*old_line), None),
            AlignedRow::NewOnly { new_line } => (None, Some(*new_line)),
        };

        // Emit top borders if this line starts a block
        let old_needs_top = old_ln.and_then(|ln| {
            old_region_map.get(&ln).and_then(|&ri| {
                if old_regions[ri].start_line == ln && !old_top_emitted.contains(&ri) {
                    Some(ri)
                } else {
                    None
                }
            })
        });
        let new_needs_top = new_ln.and_then(|ln| {
            new_region_map.get(&ln).and_then(|&ri| {
                if new_regions[ri].start_line == ln && !new_top_emitted.contains(&ri) {
                    Some(ri)
                } else {
                    None
                }
            })
        });

        // Emit top borders
        match (old_needs_top, new_needs_top) {
            (Some(ori), Some(nri)) => {
                let or = &old_regions[ori];
                let nr = &new_regions[nri];
                old_result.push(render_border_top(
                    BLOCK_COLORS[or.color_idx], &or.label, border_width, or.is_selected,
                ));
                new_result.push(render_border_top(
                    BLOCK_COLORS[nr.color_idx], &nr.label, border_width, nr.is_selected,
                ));
                old_top_emitted.insert(ori);
                new_top_emitted.insert(nri);
            }
            (Some(ori), None) => {
                let or = &old_regions[ori];
                old_result.push(render_border_top(
                    BLOCK_COLORS[or.color_idx], &or.label, border_width, or.is_selected,
                ));
                new_result.push(padding_line());
                old_top_emitted.insert(ori);
            }
            (None, Some(nri)) => {
                let nr = &new_regions[nri];
                old_result.push(padding_line());
                new_result.push(render_border_top(
                    BLOCK_COLORS[nr.color_idx], &nr.label, border_width, nr.is_selected,
                ));
                new_top_emitted.insert(nri);
            }
            (None, None) => {}
        }

        // Emit content
        match row {
            AlignedRow::Both { old_line, new_line } => {
                let old_info = get_region_info(old_regions, &old_region_map, *old_line);
                let new_info = get_region_info(new_regions, &new_region_map, *new_line);
                let ot = old_file_lines.get(*old_line - 1).copied().unwrap_or("");
                let nt = new_file_lines.get(*new_line - 1).copied().unwrap_or("");
                old_result.push(render_content_line(*old_line, ot, old_info, h_scroll));
                new_result.push(render_content_line(*new_line, nt, new_info, h_scroll));
            }
            AlignedRow::OldOnly { old_line } => {
                let old_info = get_region_info_as_changed(old_regions, &old_region_map, *old_line);
                let ot = old_file_lines.get(*old_line - 1).copied().unwrap_or("");
                old_result.push(render_content_line(*old_line, ot, old_info, h_scroll));
                new_result.push(padding_line());
            }
            AlignedRow::NewOnly { new_line } => {
                let new_info = get_region_info_as_changed(new_regions, &new_region_map, *new_line);
                let nt = new_file_lines.get(*new_line - 1).copied().unwrap_or("");
                old_result.push(padding_line());
                new_result.push(render_content_line(*new_line, nt, new_info, h_scroll));
            }
        }

        // Emit bottom borders if this line ends a block
        let old_needs_bottom = old_ln.and_then(|ln| {
            old_region_map.get(&ln).and_then(|&ri| {
                if old_regions[ri].end_line == ln && !old_bottom_emitted.contains(&ri) {
                    Some(ri)
                } else {
                    None
                }
            })
        });
        let new_needs_bottom = new_ln.and_then(|ln| {
            new_region_map.get(&ln).and_then(|&ri| {
                if new_regions[ri].end_line == ln && !new_bottom_emitted.contains(&ri) {
                    Some(ri)
                } else {
                    None
                }
            })
        });

        match (old_needs_bottom, new_needs_bottom) {
            (Some(ori), Some(nri)) => {
                let or = &old_regions[ori];
                let nr = &new_regions[nri];
                old_result.push(render_border_bottom(
                    BLOCK_COLORS[or.color_idx], border_width, or.is_selected,
                ));
                new_result.push(render_border_bottom(
                    BLOCK_COLORS[nr.color_idx], border_width, nr.is_selected,
                ));
                old_bottom_emitted.insert(ori);
                new_bottom_emitted.insert(nri);
            }
            (Some(ori), None) => {
                let or = &old_regions[ori];
                old_result.push(render_border_bottom(
                    BLOCK_COLORS[or.color_idx], border_width, or.is_selected,
                ));
                new_result.push(padding_line());
                old_bottom_emitted.insert(ori);
            }
            (None, Some(nri)) => {
                let nr = &new_regions[nri];
                old_result.push(padding_line());
                new_result.push(render_border_bottom(
                    BLOCK_COLORS[nr.color_idx], border_width, nr.is_selected,
                ));
                new_bottom_emitted.insert(nri);
            }
            (None, None) => {}
        }
    }

    (old_result, new_result)
}

// === Block region building ===

/// Map from 1-based line number to region index
fn build_region_map(regions: &[BlockRegion]) -> std::collections::HashMap<usize, usize> {
    let mut map = std::collections::HashMap::new();
    for (ri, r) in regions.iter().enumerate() {
        for line in r.start_line..=r.end_line {
            // Only store the first region for a line (in case of overlap)
            map.entry(line).or_insert(ri);
        }
    }
    map
}

fn get_region_info(
    regions: &[BlockRegion],
    region_map: &std::collections::HashMap<usize, usize>,
    line_num: usize,
) -> Option<ContentRegionInfo> {
    if let Some(&ri) = region_map.get(&line_num) {
        let r = &regions[ri];
        Some(ContentRegionInfo {
            color: BLOCK_COLORS[r.color_idx],
            is_changed: r.changed_lines.contains(&line_num),
            is_selected: r.is_selected,
        })
    } else {
        None
    }
}

/// Like get_region_info but forces is_changed=true (for diff Delete/Insert lines)
fn get_region_info_as_changed(
    regions: &[BlockRegion],
    region_map: &std::collections::HashMap<usize, usize>,
    line_num: usize,
) -> Option<ContentRegionInfo> {
    if let Some(&ri) = region_map.get(&line_num) {
        let r = &regions[ri];
        Some(ContentRegionInfo {
            color: BLOCK_COLORS[r.color_idx],
            is_changed: true,
            is_selected: r.is_selected,
        })
    } else {
        None
    }
}

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

        let kind = classify_block_kind(&change.kind);
        let old_label = build_label(change, kind, true);
        let new_label = build_label(change, kind, false);

        if let Some(ref sym) = change.old_symbol {
            if old_file.map_or(false, |f| sym.file_path == f) {
                old_regions.push(BlockRegion {
                    start_line: sym.line_range.0,
                    end_line: sym.line_range.1,
                    color_idx,
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

// === Labels and classification ===

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockKind {
    Moved,
    Added,
    Deleted,
    Modified,
    Extracted,
    Inlined,
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
                format!("{} -> {}", name, dest)
            } else {
                let src = change
                    .old_symbol
                    .as_ref()
                    .map(|s| s.file_path.display().to_string())
                    .unwrap_or_default();
                format!("{} <- {}", name, src)
            }
        }
        BlockKind::Extracted => {
            if is_old { format!("{} -> extracted", name) }
            else { format!("{} <- extracted", name) }
        }
        BlockKind::Inlined => {
            if is_old { format!("{} -> inlined", name) }
            else { format!("{} <- inlined", name) }
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

// === Scroll ===

fn find_scroll_vline(vlines: &[Line], file_line_scroll: usize) -> usize {
    if file_line_scroll == 0 {
        return 0;
    }
    let mut content_count = 0usize;
    for (i, line) in vlines.iter().enumerate() {
        if is_content_vline(line) {
            if content_count == file_line_scroll {
                return i.saturating_sub(1);
            }
            content_count += 1;
        }
    }
    vlines.len().saturating_sub(1)
}

fn is_content_vline(line: &Line) -> bool {
    if line.spans.is_empty() { return false; }
    let first = &line.spans[0].content;
    if first.is_empty() { return false; }
    let ch = first.chars().next().unwrap_or(' ');
    ch != '┌' && ch != '└'
}

// === Line rendering ===

#[derive(Debug, Clone)]
struct ContentRegionInfo {
    color: Color,
    is_changed: bool,
    is_selected: bool,
}

fn padding_line<'a>() -> Line<'a> {
    Line::from(Span::styled("", Style::default().bg(Color::Rgb(20, 20, 30))))
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
    h_scroll: usize,
) -> Line<'a> {
    let expanded = expand_tabs(text);
    let display_text: String = expanded.chars().skip(h_scroll).collect();
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
        spans.push(Span::styled(display_text, text_style));
    } else {
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(
            format!("{:4} ", line_num),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(display_text, Style::default().fg(Color::Gray)));
    }

    Line::from(spans)
}

fn expand_tabs(text: &str) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut col = 0;
    for ch in text.chars() {
        if ch == '\t' {
            let spaces = 4 - (col % 4);
            for _ in 0..spaces { result.push(' '); }
            col += spaces;
        } else {
            result.push(ch);
            col += 1;
        }
    }
    result
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
