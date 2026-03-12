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

use crate::tui::theme::BLOCK_COLORS;

/// Map from 1-based line number to list of region indices (outermost first, innermost last)
type RegionMap = std::collections::HashMap<usize, Vec<usize>>;

/// Render side-by-side view inline (called from detail panel)
pub fn render_inline(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(change) = app.selected_change() else {
        let p = Paragraph::new("No change selected");
        f.render_widget(p, area);
        return;
    };

    let old_file = change.old_symbol.as_ref().map(|s| s.file_path.clone());
    let new_file = change.new_symbol.as_ref().map(|s| s.file_path.clone());

    // Detect pure-add or pure-delete files (no cross-file moves involved)
    let single_column_mode = detect_single_column_mode(change, &app.diff_result.changes);

    if single_column_mode != SingleColumnMode::None {
        render_single_column(f, area, app, single_column_mode);
        return;
    }

    // For DEL (no new_symbol) or ADD (no old_symbol), use the other side's path
    // so both panels show the file for context
    let display_old_file = old_file.clone().or_else(|| new_file.clone());
    let display_new_file = new_file.clone().or_else(|| old_file.clone());

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let header_halves =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[0]);

    let old_path_str = display_old_file
        .as_ref()
        .map(|p| format!(" <- {}", p.display()))
        .unwrap_or_else(|| " <- (none)".to_string());
    let new_path_str = display_new_file
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
    // For DEL/ADD, load both sides using the available file path for context
    let old_content = display_old_file
        .as_ref()
        .and_then(|p| app.load_old_file_content(p))
        .map(|s| s.to_string());
    let new_content = display_new_file
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
    // detail_scroll is old-side based, so use old_vlines for scroll computation.
    // Falls back to new_vlines for ADD changes (old side empty).
    let scroll_vline = if old_text.is_empty() {
        find_scroll_vline(&new_vlines, scroll)
    } else {
        find_scroll_vline(&old_vlines, scroll)
    };

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

// === Single-column rendering for pure ADD/DEL files ===

#[derive(Debug, Clone, Copy, PartialEq)]
enum SingleColumnMode {
    None,
    Added,   // Pure new file — show full width
    Deleted, // Pure deleted file — show full width
}

/// Check if the selected change's file is a pure-add or pure-delete file.
/// A file is "pure add" if all changes for that file path are Added (no moves into it).
/// A file is "pure delete" if all changes for that file path are Deleted (no moves from it).
fn detect_single_column_mode(
    selected: &SemanticChange,
    all_changes: &[SemanticChange],
) -> SingleColumnMode {
    // Only applies to ADD or DEL changes
    let (target_file, candidate_mode) = match &selected.kind {
        ChangeKind::Added => {
            if let Some(ref sym) = selected.new_symbol {
                (&sym.file_path, SingleColumnMode::Added)
            } else {
                return SingleColumnMode::None;
            }
        }
        ChangeKind::Deleted => {
            if let Some(ref sym) = selected.old_symbol {
                (&sym.file_path, SingleColumnMode::Deleted)
            } else {
                return SingleColumnMode::None;
            }
        }
        _ => return SingleColumnMode::None,
    };

    // Check all changes involving this file
    for change in all_changes {
        match candidate_mode {
            SingleColumnMode::Added => {
                // Check if any change has new_symbol in this file that isn't Added
                if let Some(ref sym) = change.new_symbol {
                    if sym.file_path == *target_file && !matches!(change.kind, ChangeKind::Added) {
                        return SingleColumnMode::None;
                    }
                }
            }
            SingleColumnMode::Deleted => {
                // Check if any change has old_symbol in this file that isn't Deleted
                if let Some(ref sym) = change.old_symbol {
                    if sym.file_path == *target_file && !matches!(change.kind, ChangeKind::Deleted)
                    {
                        return SingleColumnMode::None;
                    }
                }
            }
            SingleColumnMode::None => unreachable!(),
        }
    }

    candidate_mode
}

/// Render a single-column view for pure ADD or DEL files.
/// Shows full-width file content with block borders for each symbol.
fn render_single_column(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    mode: SingleColumnMode,
) {
    let change = app.selected_change().unwrap();
    let file_path = match mode {
        SingleColumnMode::Added => change.new_symbol.as_ref().map(|s| s.file_path.clone()),
        SingleColumnMode::Deleted => change.old_symbol.as_ref().map(|s| s.file_path.clone()),
        SingleColumnMode::None => unreachable!(),
    };

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);

    // Header
    let (label, header_color) = match mode {
        SingleColumnMode::Added => ("+ (new file)", Color::Green),
        SingleColumnMode::Deleted => ("- (deleted file)", Color::Red),
        SingleColumnMode::None => unreachable!(),
    };
    let path_str = file_path
        .as_ref()
        .map(|p| format!(" {} {}", label, p.display()))
        .unwrap_or_else(|| format!(" {}", label));
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            path_str,
            Style::default().fg(header_color).add_modifier(Modifier::BOLD),
        ))),
        chunks[0],
    );

    // Load file content
    let file_content = match mode {
        SingleColumnMode::Added => file_path
            .as_ref()
            .and_then(|p| app.load_file_content(p))
            .map(|s| s.to_string()),
        SingleColumnMode::Deleted => file_path
            .as_ref()
            .and_then(|p| app.load_old_file_content(p))
            .map(|s| s.to_string()),
        SingleColumnMode::None => unreachable!(),
    };
    let file_fallback = match mode {
        SingleColumnMode::Added => app
            .selected_change()
            .and_then(|c| c.new_symbol.as_ref())
            .map(|s| s.body_text.clone()),
        SingleColumnMode::Deleted => app
            .selected_change()
            .and_then(|c| c.old_symbol.as_ref())
            .map(|s| s.body_text.clone()),
        SingleColumnMode::None => unreachable!(),
    };
    let file_text = file_content
        .as_deref()
        .or(file_fallback.as_deref())
        .unwrap_or("");

    // Build block regions for this file
    let all_changes: Vec<&SemanticChange> = app.diff_result.changes.iter().collect();
    let selected_idx = app.selected_index;
    let regions = match mode {
        SingleColumnMode::Added => {
            let (_, new_regions) = build_block_regions(
                &all_changes,
                None,
                file_path.as_deref(),
                selected_idx,
            );
            new_regions
        }
        SingleColumnMode::Deleted => {
            let (old_regions, _) = build_block_regions(
                &all_changes,
                file_path.as_deref(),
                None,
                selected_idx,
            );
            old_regions
        }
        SingleColumnMode::None => unreachable!(),
    };

    let scroll = app.detail_scroll;
    let h_scroll = app.detail_h_scroll;
    let panel_width = chunks[1].width as usize;
    let visible_height = chunks[1].height as usize;
    let border_width = panel_width.saturating_sub(1);

    // Render all lines with block borders
    let file_lines: Vec<&str> = file_text.lines().collect();
    let region_map = build_region_map(&regions);
    let mut vlines: Vec<Line> = Vec::new();

    let mut top_emitted: HashSet<usize> = HashSet::new();
    let mut bottom_emitted: HashSet<usize> = HashSet::new();

    for line_num in 1..=file_lines.len() {
        // Top borders
        let starting = starting_regions(&regions, &region_map, Some(line_num), &top_emitted);
        for &ri in &starting {
            let prefix = border_nesting_prefix(&regions, &region_map, line_num, ri);
            let r = &regions[ri];
            top_emitted.insert(ri);
            vlines.push(render_border_top(
                prefix,
                BLOCK_COLORS[r.color_idx],
                &r.label,
                border_width,
                r.is_selected,
            ));
        }

        // Content line
        let prefix = nesting_prefix(&regions, &region_map, line_num);
        let info = get_region_info(&regions, &region_map, line_num);
        // Don't highlight content — just show border box
        let info = info.map(|mut i| {
            i.is_changed = false;
            i
        });
        let text = file_lines.get(line_num - 1).copied().unwrap_or("");
        vlines.push(render_content_line(line_num, text, info, prefix, h_scroll));

        // Bottom borders
        let ending = ending_regions(&regions, &region_map, Some(line_num), &bottom_emitted);
        for &ri in &ending {
            let prefix = border_nesting_prefix(&regions, &region_map, line_num, ri);
            let r = &regions[ri];
            bottom_emitted.insert(ri);
            vlines.push(render_border_bottom(
                prefix,
                BLOCK_COLORS[r.color_idx],
                border_width,
                r.is_selected,
            ));
        }
    }

    // Scroll
    let scroll_vline = find_scroll_vline(&vlines, scroll);
    let mut visible: Vec<Line> = Vec::with_capacity(visible_height);
    for i in scroll_vline..(scroll_vline + visible_height) {
        visible.push(vlines.get(i).cloned().unwrap_or_else(tilde_line));
    }

    f.render_widget(Paragraph::new(visible), chunks[1]);
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

// === Phase 2: Render aligned rows with nested block borders ===

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

        // Collect all regions that need top borders at this line
        let old_starting = starting_regions(old_regions, &old_region_map, old_ln, &old_top_emitted);
        let new_starting = starting_regions(new_regions, &new_region_map, new_ln, &new_top_emitted);

        // Emit top borders (may be multiple if nested blocks start at same line)
        let max_tops = old_starting.len().max(new_starting.len());
        for bi in 0..max_tops {
            let old_border = if let Some(&ri) = old_starting.get(bi) {
                let prefix = border_nesting_prefix(old_regions, &old_region_map, old_ln.unwrap(), ri);
                let r = &old_regions[ri];
                old_top_emitted.insert(ri);
                render_border_top(prefix, BLOCK_COLORS[r.color_idx], &r.label, border_width, r.is_selected)
            } else {
                padding_line()
            };
            let new_border = if let Some(&ri) = new_starting.get(bi) {
                let prefix = border_nesting_prefix(new_regions, &new_region_map, new_ln.unwrap(), ri);
                let r = &new_regions[ri];
                new_top_emitted.insert(ri);
                render_border_top(prefix, BLOCK_COLORS[r.color_idx], &r.label, border_width, r.is_selected)
            } else {
                padding_line()
            };
            old_result.push(old_border);
            new_result.push(new_border);
        }

        // Emit content
        match row {
            AlignedRow::Both { old_line, new_line } => {
                let old_prefix = nesting_prefix(old_regions, &old_region_map, *old_line);
                let new_prefix = nesting_prefix(new_regions, &new_region_map, *new_line);
                let old_info = get_region_info(old_regions, &old_region_map, *old_line);
                let new_info = get_region_info(new_regions, &new_region_map, *new_line);
                let ot = old_file_lines.get(*old_line - 1).copied().unwrap_or("");
                let nt = new_file_lines.get(*new_line - 1).copied().unwrap_or("");
                // For Both rows: use word-level diff when text differs within a block,
                // otherwise just show the border box without background highlighting
                let in_block = old_info.is_some() && new_info.is_some();
                if in_block && ot != nt {
                    let (old_spans, new_spans) = render_word_diff_line(
                        *old_line,
                        *new_line,
                        ot,
                        nt,
                        old_info.as_ref().unwrap(),
                        new_info.as_ref().unwrap(),
                        old_prefix,
                        new_prefix,
                        h_scroll,
                    );
                    old_result.push(old_spans);
                    new_result.push(new_spans);
                } else {
                    // Both rows never get bg highlight — border only.
                    // Highlight is reserved for OldOnly/NewOnly (actual diff lines).
                    let old_clean = old_info.map(|mut i| { i.is_changed = false; i });
                    let new_clean = new_info.map(|mut i| { i.is_changed = false; i });
                    old_result.push(render_content_line(*old_line, ot, old_clean, old_prefix, h_scroll));
                    new_result.push(render_content_line(*new_line, nt, new_clean, new_prefix, h_scroll));
                }
            }
            AlignedRow::OldOnly { old_line } => {
                let old_prefix = nesting_prefix(old_regions, &old_region_map, *old_line);
                let old_info = get_region_info(old_regions, &old_region_map, *old_line);
                let ot = old_file_lines.get(*old_line - 1).copied().unwrap_or("");
                // Inside a block: only highlight if body_diff marks this line as changed.
                // Outside blocks: use a dim red to indicate file-level deletion.
                let old_info = if old_info.as_ref().map_or(false, |i| !i.is_changed) {
                    // Inside block, not changed → border only
                    old_info
                } else if old_info.is_some() {
                    // Inside block, changed → use block color
                    get_region_info_as_changed(old_regions, &old_region_map, *old_line)
                } else {
                    // Outside any block → dim red (file-level diff line)
                    Some(ContentRegionInfo {
                        color: Color::DarkGray,
                        is_changed: true,
                        is_selected: false,
                    })
                };
                old_result.push(render_content_line(*old_line, ot, old_info, old_prefix, h_scroll));
                new_result.push(padding_line());
            }
            AlignedRow::NewOnly { new_line } => {
                let new_prefix = nesting_prefix(new_regions, &new_region_map, *new_line);
                let new_info = get_region_info(new_regions, &new_region_map, *new_line);
                let nt = new_file_lines.get(*new_line - 1).copied().unwrap_or("");
                let new_info = if new_info.as_ref().map_or(false, |i| !i.is_changed) {
                    new_info
                } else if new_info.is_some() {
                    get_region_info_as_changed(new_regions, &new_region_map, *new_line)
                } else {
                    Some(ContentRegionInfo {
                        color: Color::DarkGray,
                        is_changed: true,
                        is_selected: false,
                    })
                };
                old_result.push(padding_line());
                new_result.push(render_content_line(*new_line, nt, new_info, new_prefix, h_scroll));
            }
        }

        // Collect all regions that need bottom borders at this line (innermost first)
        let old_ending = ending_regions(old_regions, &old_region_map, old_ln, &old_bottom_emitted);
        let new_ending = ending_regions(new_regions, &new_region_map, new_ln, &new_bottom_emitted);

        let max_bottoms = old_ending.len().max(new_ending.len());
        for bi in 0..max_bottoms {
            let old_border = if let Some(&ri) = old_ending.get(bi) {
                let prefix = border_nesting_prefix(old_regions, &old_region_map, old_ln.unwrap(), ri);
                let r = &old_regions[ri];
                old_bottom_emitted.insert(ri);
                render_border_bottom(prefix, BLOCK_COLORS[r.color_idx], border_width, r.is_selected)
            } else {
                padding_line()
            };
            let new_border = if let Some(&ri) = new_ending.get(bi) {
                let prefix = border_nesting_prefix(new_regions, &new_region_map, new_ln.unwrap(), ri);
                let r = &new_regions[ri];
                new_bottom_emitted.insert(ri);
                render_border_bottom(prefix, BLOCK_COLORS[r.color_idx], border_width, r.is_selected)
            } else {
                padding_line()
            };
            old_result.push(old_border);
            new_result.push(new_border);
        }
    }

    (old_result, new_result)
}

/// Collect region indices that start at the given line (outermost first)
fn starting_regions(
    regions: &[BlockRegion],
    region_map: &RegionMap,
    line_num: Option<usize>,
    top_emitted: &HashSet<usize>,
) -> Vec<usize> {
    line_num.map_or(vec![], |ln| {
        region_map.get(&ln).map_or(vec![], |stack| {
            stack
                .iter()
                .filter(|&&ri| regions[ri].start_line == ln && !top_emitted.contains(&ri))
                .copied()
                .collect()
        })
    })
}

/// Collect region indices that end at the given line (innermost first)
fn ending_regions(
    regions: &[BlockRegion],
    region_map: &RegionMap,
    line_num: Option<usize>,
    bottom_emitted: &HashSet<usize>,
) -> Vec<usize> {
    line_num.map_or(vec![], |ln| {
        region_map.get(&ln).map_or(vec![], |stack| {
            stack
                .iter()
                .rev() // innermost first
                .filter(|&&ri| regions[ri].end_line == ln && !bottom_emitted.contains(&ri))
                .copied()
                .collect()
        })
    })
}

// === Block region building ===

/// Build a map from 1-based line number to list of region indices.
/// Each line's regions are sorted outermost (largest) first, innermost (smallest) last.
fn build_region_map(regions: &[BlockRegion]) -> RegionMap {
    let mut map: RegionMap = std::collections::HashMap::new();
    for (ri, r) in regions.iter().enumerate() {
        for line in r.start_line..=r.end_line {
            map.entry(line).or_default().push(ri);
        }
    }
    // Sort: largest region first (outermost), smallest last (innermost)
    for v in map.values_mut() {
        v.sort_by_key(|&ri| {
            std::cmp::Reverse(regions[ri].end_line.saturating_sub(regions[ri].start_line))
        });
    }
    map
}

/// Get region info for the innermost region at a line
fn get_region_info(
    regions: &[BlockRegion],
    region_map: &RegionMap,
    line_num: usize,
) -> Option<ContentRegionInfo> {
    region_map
        .get(&line_num)
        .and_then(|stack| stack.last())
        .map(|&ri| {
            let r = &regions[ri];
            ContentRegionInfo {
                color: BLOCK_COLORS[r.color_idx],
                is_changed: r.changed_lines.contains(&line_num),
                is_selected: r.is_selected,
            }
        })
}

/// Like get_region_info but forces is_changed=true (for diff Delete/Insert lines)
fn get_region_info_as_changed(
    regions: &[BlockRegion],
    region_map: &RegionMap,
    line_num: usize,
) -> Option<ContentRegionInfo> {
    region_map
        .get(&line_num)
        .and_then(|stack| stack.last())
        .map(|&ri| {
            let r = &regions[ri];
            ContentRegionInfo {
                color: BLOCK_COLORS[r.color_idx],
                is_changed: true,
                is_selected: r.is_selected,
            }
        })
}

/// Build nesting prefix spans for content lines: │ for each outer region (excluding innermost)
fn nesting_prefix<'a>(
    regions: &[BlockRegion],
    region_map: &RegionMap,
    line_num: usize,
) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    if let Some(stack) = region_map.get(&line_num) {
        // All regions except the last (innermost) contribute a │ border
        for &ri in &stack[..stack.len().saturating_sub(1)] {
            let r = &regions[ri];
            let color = BLOCK_COLORS[r.color_idx];
            let style = if r.is_selected {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            spans.push(Span::styled("│", style));
        }
    }
    spans
}

/// Build nesting prefix spans for a border line: │ for each region outermost to (but not including) border_ri
fn border_nesting_prefix<'a>(
    regions: &[BlockRegion],
    region_map: &RegionMap,
    line_num: usize,
    border_ri: usize,
) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    if let Some(stack) = region_map.get(&line_num) {
        for &ri in stack {
            if ri == border_ri {
                break;
            }
            let r = &regions[ri];
            let color = BLOCK_COLORS[r.color_idx];
            let style = if r.is_selected {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            spans.push(Span::styled("│", style));
        }
    }
    spans
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
    Renamed,
    Signature,
    Visibility,
}

fn classify_block_kind(kind: &ChangeKind) -> BlockKind {
    match kind {
        ChangeKind::Added => BlockKind::Added,
        ChangeKind::Deleted => BlockKind::Deleted,
        ChangeKind::Moved { .. } | ChangeKind::MovedAndModified { .. } => BlockKind::Moved,
        ChangeKind::Extracted { .. } => BlockKind::Extracted,
        ChangeKind::Inlined { .. } => BlockKind::Inlined,
        ChangeKind::Renamed { .. } => BlockKind::Renamed,
        ChangeKind::SignatureChanged { .. } => BlockKind::Signature,
        ChangeKind::VisibilityChanged { .. } => BlockKind::Visibility,
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
        BlockKind::Renamed => {
            if let ChangeKind::Renamed { old_name, new_name } = &change.kind {
                if is_old {
                    format!("REN {} -> {}", old_name, new_name)
                } else {
                    format!("REN {} -> {}", old_name, new_name)
                }
            } else {
                format!("~ {}", name)
            }
        }
        BlockKind::Signature => {
            let desc = change.kind.short_description();
            format!("SIG {} ({})", name, desc)
        }
        BlockKind::Visibility => {
            if let ChangeKind::VisibilityChanged { old, new } = &change.kind {
                format!("VIS {} ({} -> {})", name, old, new)
            } else {
                format!("~ {}", name)
            }
        }
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

/// A content line is one that is NOT a border line.
/// Border lines contain ┌ or └ characters (box-drawing).
fn is_content_vline(line: &Line) -> bool {
    if line.spans.is_empty() {
        return false;
    }
    for span in &line.spans {
        for ch in span.content.chars() {
            if ch == '┌' || ch == '└' {
                return false;
            }
        }
    }
    line.spans.iter().any(|s| !s.content.is_empty())
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

fn render_border_top<'a>(
    prefix: Vec<Span<'a>>,
    color: Color,
    label: &str,
    width: usize,
    is_selected: bool,
) -> Line<'a> {
    let style = if is_selected {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    let prefix_len = prefix.len();
    let adjusted_width = width.saturating_sub(prefix_len);
    let pfx = "┌─ ";
    let suffix = " ─┐";
    let max_label = adjusted_width.saturating_sub(pfx.len() + suffix.len() + 1);
    let label_trimmed: String = label.chars().take(max_label).collect();
    let fill_len = adjusted_width.saturating_sub(
        pfx.chars().count() + label_trimmed.chars().count() + suffix.chars().count(),
    );
    let fill: String = "─".repeat(fill_len);
    let mut spans = prefix;
    spans.push(Span::styled(
        format!("{}{}{}{}", pfx, label_trimmed, fill, suffix),
        style,
    ));
    Line::from(spans)
}

fn render_border_bottom<'a>(
    prefix: Vec<Span<'a>>,
    color: Color,
    width: usize,
    is_selected: bool,
) -> Line<'a> {
    let style = if is_selected {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    let prefix_len = prefix.len();
    let adjusted_width = width.saturating_sub(prefix_len);
    let fill_len = adjusted_width.saturating_sub(2);
    let fill: String = "─".repeat(fill_len);
    let mut spans = prefix;
    spans.push(Span::styled(format!("└{}┘", fill), style));
    Line::from(spans)
}

fn render_content_line<'a>(
    line_num: usize,
    text: &str,
    region_info: Option<ContentRegionInfo>,
    prefix: Vec<Span<'a>>,
    h_scroll: usize,
) -> Line<'a> {
    let expanded = expand_tabs(text);
    let display_text: String = expanded.chars().skip(h_scroll).collect();
    let mut spans: Vec<Span<'a>> = prefix;

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

/// Render a pair of lines with word-level diff highlighting.
/// Unchanged words get the block's normal style; changed words get bold + bg highlight.
fn render_word_diff_line<'a>(
    old_num: usize,
    new_num: usize,
    old_text: &str,
    new_text: &str,
    old_info: &ContentRegionInfo,
    new_info: &ContentRegionInfo,
    old_prefix: Vec<Span<'a>>,
    new_prefix: Vec<Span<'a>>,
    h_scroll: usize,
) -> (Line<'a>, Line<'a>) {
    use similar::{ChangeTag, TextDiff};

    let old_expanded = expand_tabs(old_text);
    let new_expanded = expand_tabs(new_text);

    let diff = TextDiff::from_words(&old_expanded, &new_expanded);

    // Collect old-side and new-side fragments
    let mut old_frags: Vec<(String, bool)> = Vec::new(); // (text, is_changed)
    let mut new_frags: Vec<(String, bool)> = Vec::new();

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                let t = change.value().to_string();
                old_frags.push((t.clone(), false));
                new_frags.push((t, false));
            }
            ChangeTag::Delete => {
                old_frags.push((change.value().to_string(), true));
            }
            ChangeTag::Insert => {
                new_frags.push((change.value().to_string(), true));
            }
        }
    }

    let old_line = build_word_diff_spans(old_num, &old_frags, old_info, old_prefix, h_scroll);
    let new_line = build_word_diff_spans(new_num, &new_frags, new_info, new_prefix, h_scroll);
    (old_line, new_line)
}

fn build_word_diff_spans<'a>(
    line_num: usize,
    frags: &[(String, bool)],
    info: &ContentRegionInfo,
    prefix: Vec<Span<'a>>,
    h_scroll: usize,
) -> Line<'a> {
    let border_style = if info.is_selected {
        Style::default()
            .fg(info.color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(info.color)
    };
    let unchanged_style = Style::default().fg(Color::Rgb(180, 180, 180));
    let changed_style = Style::default()
        .fg(Color::White)
        .bg(color_to_bg(info.color))
        .add_modifier(Modifier::BOLD);
    let num_style = Style::default()
        .fg(info.color)
        .add_modifier(Modifier::BOLD);

    // Flatten all fragments into a single string, then apply h_scroll
    let full_text: String = frags.iter().map(|(t, _)| t.as_str()).collect();
    let scrolled: String = full_text.chars().skip(h_scroll).collect();

    // Map character positions to changed/unchanged
    let mut char_changed: Vec<bool> = Vec::new();
    for (text, is_changed) in frags {
        for _ in text.chars() {
            char_changed.push(*is_changed);
        }
    }
    // Skip h_scroll chars
    let char_changed: Vec<bool> = char_changed.into_iter().skip(h_scroll).collect();

    // Build spans with nesting prefix
    let mut spans: Vec<Span<'a>> = prefix;
    spans.push(Span::styled("│", border_style));
    spans.push(Span::styled(format!("{:4} ", line_num), num_style));

    let chars: Vec<char> = scrolled.chars().collect();
    if chars.is_empty() {
        return Line::from(spans);
    }

    let mut i = 0;
    while i < chars.len() {
        let is_changed = char_changed.get(i).copied().unwrap_or(false);
        let mut j = i + 1;
        while j < chars.len()
            && char_changed.get(j).copied().unwrap_or(false) == is_changed
        {
            j += 1;
        }
        let segment: String = chars[i..j].iter().collect();
        let style = if is_changed {
            changed_style
        } else {
            unchanged_style
        };
        spans.push(Span::styled(segment, style));
        i = j;
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
