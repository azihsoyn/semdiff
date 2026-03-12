use crate::diff::body_diff::DiffLineTag;
use crate::diff::change::{ChangeKind, DiffResult, SemanticChange};
use crate::llm::review::ReviewResult;
use crate::repo::RepoAnalysis;
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelFocus {
    Summary,
    Detail,
    Review,
    Impact,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BottomPanel {
    Review,
    Impact,
}

pub struct App {
    pub diff_result: DiffResult,
    pub selected_index: usize,
    pub panel_focus: PanelFocus,
    pub bottom_panel: BottomPanel,
    pub bottom_visible: bool,
    pub reviews: HashMap<usize, ReviewResult>,
    pub overall_review: Option<ReviewResult>,
    pub repo_analysis: Option<RepoAnalysis>,
    pub summary_scroll: usize,
    pub detail_scroll: usize,
    pub detail_h_scroll: usize,
    pub bottom_scroll: usize,
    pub should_quit: bool,
    pub llm_enabled: bool,
    pub loading_review: bool,
    pub status_message: Option<String>,
    /// Root directory for loading full file contents (new/current version)
    pub repo_root: Option<PathBuf>,
    /// Root directory for old version files (for side-by-side view in dirs mode)
    pub old_root: Option<PathBuf>,
    /// Cache of loaded file contents
    pub file_cache: HashMap<PathBuf, String>,
    /// Persistent list state for summary panel (tracks scroll offset)
    pub summary_list_state: ListState,
    /// Navigation order: change indices ordered by file grouping (matches visual order)
    pub nav_order: Vec<usize>,
    /// Current position in nav_order
    pub nav_pos: usize,
    /// Collapsed file groups in summary panel (file_info string → collapsed)
    pub collapsed_files: std::collections::HashSet<String>,
}

impl App {
    pub fn new(
        diff_result: DiffResult,
        llm_enabled: bool,
        repo_analysis: Option<RepoAnalysis>,
    ) -> Self {
        let bottom_visible = repo_analysis.is_some();
        let bottom_panel = if repo_analysis.is_some() {
            BottomPanel::Impact
        } else {
            BottomPanel::Review
        };

        // Build navigation order grouped by file (matches summary panel display order)
        let nav_order = build_nav_order(&diff_result);

        // selected_index = first change in nav order
        let selected_index = nav_order.first().copied().unwrap_or(0);

        // Auto-scroll to first change's location (old-side based)
        let initial_scroll = diff_result
            .changes
            .get(selected_index)
            .and_then(|c| c.old_symbol.as_ref().or(c.new_symbol.as_ref()))
            .map(|s| s.line_range.0.saturating_sub(3))
            .unwrap_or(0);

        Self {
            diff_result,
            selected_index,
            panel_focus: PanelFocus::Summary,
            bottom_panel,
            bottom_visible,
            reviews: HashMap::new(),
            overall_review: None,
            repo_analysis,
            summary_scroll: 0,
            detail_scroll: initial_scroll,
            detail_h_scroll: 0,
            bottom_scroll: 0,
            should_quit: false,
            llm_enabled,
            loading_review: false,
            status_message: None,
            repo_root: None,
            old_root: None,
            file_cache: HashMap::new(),
            summary_list_state: ListState::default(),
            nav_order,
            nav_pos: 0,
            collapsed_files: std::collections::HashSet::new(),
        }
    }

    pub fn selected_change(&self) -> Option<&SemanticChange> {
        self.diff_result.changes.get(self.selected_index)
    }

    pub fn change_count(&self) -> usize {
        self.diff_result.changes.len()
    }

    pub fn select_next(&mut self) {
        self.select_next_visible();
    }

    pub fn select_prev(&mut self) {
        self.select_prev_visible();
    }

    /// Auto-scroll detail panel to the first changed line within the selected block.
    /// Uses old-side line numbers since the aligned view preserves old line order.
    /// Falls back to new_symbol if no old_symbol exists (ADD changes).
    pub fn auto_scroll_detail(&mut self) {
        if let Some(change) = self.diff_result.changes.get(self.selected_index) {
            let sym = change.old_symbol.as_ref().or(change.new_symbol.as_ref());
            if let Some(sym) = sym {
                // Try to find the first changed line within the block (old-side)
                let first_changed = find_first_changed_line(change, sym.line_range.0);
                self.detail_scroll = first_changed.saturating_sub(5);
                self.detail_h_scroll = 0;
                return;
            }
        }
        self.detail_scroll = 0;
        self.detail_h_scroll = 0;
    }

    pub fn scroll_detail_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(1);
        self.sync_selection_from_scroll();
    }

    pub fn scroll_detail_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
        self.sync_selection_from_scroll();
    }

    /// When scrolling in the detail panel, if the viewport enters a different
    /// block's line range, auto-select that block in the summary panel.
    /// Uses old-side line numbers (matching detail_scroll).
    /// Prefers the most specific (smallest) block when multiple blocks overlap.
    fn sync_selection_from_scroll(&mut self) {
        // Determine the file currently being viewed (prefer old side to match scroll)
        let current_file = self
            .selected_change()
            .and_then(|c| {
                c.old_symbol
                    .as_ref()
                    .or(c.new_symbol.as_ref())
                    .map(|s| s.file_path.clone())
            });
        let Some(current_file) = current_file else {
            return;
        };

        // Use a line a few rows into the visible area as the "probe" line
        let probe_line = self.detail_scroll + 5;

        // Find the most specific (smallest range) change whose line range contains the probe line
        // Use old_symbol line ranges since detail_scroll is old-side based
        let mut best: Option<(usize, usize)> = None; // (change_index, range_size)
        for (i, change) in self.diff_result.changes.iter().enumerate() {
            if i == self.selected_index {
                continue;
            }
            let sym = change
                .old_symbol
                .as_ref()
                .or(change.new_symbol.as_ref());
            if let Some(sym) = sym {
                if sym.file_path == current_file
                    && probe_line >= sym.line_range.0
                    && probe_line <= sym.line_range.1
                {
                    let size = sym.line_range.1 - sym.line_range.0;
                    if best.map_or(true, |(_, best_size)| size < best_size) {
                        best = Some((i, size));
                    }
                }
            }
        }

        if let Some((i, _)) = best {
            self.selected_index = i;
            if let Some(pos) = self.nav_order.iter().position(|&idx| idx == i) {
                self.nav_pos = pos;
            }
            self.bottom_scroll = 0;
        }
    }

    pub fn scroll_detail_right(&mut self) {
        self.detail_h_scroll = self.detail_h_scroll.saturating_add(4);
    }

    pub fn scroll_detail_left(&mut self) {
        self.detail_h_scroll = self.detail_h_scroll.saturating_sub(4);
    }

    pub fn scroll_bottom_down(&mut self) {
        self.bottom_scroll = self.bottom_scroll.saturating_add(1);
    }

    pub fn scroll_bottom_up(&mut self) {
        self.bottom_scroll = self.bottom_scroll.saturating_sub(1);
    }

    pub fn cycle_focus(&mut self) {
        self.panel_focus = match self.panel_focus {
            PanelFocus::Summary => PanelFocus::Detail,
            PanelFocus::Detail => {
                if self.bottom_visible {
                    match self.bottom_panel {
                        BottomPanel::Review => PanelFocus::Review,
                        BottomPanel::Impact => PanelFocus::Impact,
                    }
                } else {
                    PanelFocus::Summary
                }
            }
            PanelFocus::Review | PanelFocus::Impact => PanelFocus::Summary,
        };
    }

    pub fn toggle_bottom(&mut self) {
        self.bottom_visible = !self.bottom_visible;
        if !self.bottom_visible
            && (self.panel_focus == PanelFocus::Review
                || self.panel_focus == PanelFocus::Impact)
        {
            self.panel_focus = PanelFocus::Summary;
        }
    }

    pub fn cycle_bottom_panel(&mut self) {
        self.bottom_panel = match self.bottom_panel {
            BottomPanel::Review => BottomPanel::Impact,
            BottomPanel::Impact => BottomPanel::Review,
        };
        self.bottom_scroll = 0;
        if self.panel_focus == PanelFocus::Review || self.panel_focus == PanelFocus::Impact {
            self.panel_focus = match self.bottom_panel {
                BottomPanel::Review => PanelFocus::Review,
                BottomPanel::Impact => PanelFocus::Impact,
            };
        }
    }

    pub fn current_review(&self) -> Option<&ReviewResult> {
        self.reviews.get(&self.selected_index)
    }

    pub fn has_repo_analysis(&self) -> bool {
        self.repo_analysis.is_some()
    }

    /// Get the file key for a given change index
    fn file_key_for_change(&self, idx: usize) -> Option<String> {
        self.diff_result.changes.get(idx).map(|c| c.file_info())
    }

    /// Toggle collapse state for the file group of the currently selected change.
    pub fn toggle_file_collapse(&mut self) {
        if let Some(file_key) = self.file_key_for_change(self.selected_index) {
            if self.collapsed_files.contains(&file_key) {
                self.collapsed_files.remove(&file_key);
            } else {
                self.collapsed_files.insert(file_key);
            }
        }
    }

    /// Move to the next visible item in nav_order (skipping collapsed items).
    pub fn select_next_visible(&mut self) {
        let mut pos = self.nav_pos + 1;
        while pos < self.nav_order.len() {
            let idx = self.nav_order[pos];
            if !self.is_collapsed_nav(idx) {
                self.nav_pos = pos;
                self.selected_index = idx;
                self.auto_scroll_detail();
                self.bottom_scroll = 0;
                return;
            }
            pos += 1;
        }
    }

    /// Move to the previous visible item in nav_order (skipping collapsed items).
    pub fn select_prev_visible(&mut self) {
        if self.nav_pos == 0 {
            return;
        }
        let mut pos = self.nav_pos - 1;
        loop {
            let idx = self.nav_order[pos];
            if !self.is_collapsed_nav(idx) {
                self.nav_pos = pos;
                self.selected_index = idx;
                self.auto_scroll_detail();
                self.bottom_scroll = 0;
                return;
            }
            if pos == 0 {
                break;
            }
            pos -= 1;
        }
    }

    /// Check if a change index belongs to a collapsed file group,
    /// but allow the first change in each file group to remain visible (as the representative).
    /// Used by navigation (select_next/prev) to skip hidden items.
    pub fn is_collapsed_nav(&self, idx: usize) -> bool {
        if let Some(file_key) = self.file_key_for_change(idx) {
            if self.collapsed_files.contains(&file_key) {
                // Allow the first change in this file group to be visible
                return !self.is_first_in_file_group(idx, &file_key);
            }
        }
        false
    }

    /// Check if this change index is the first one in its file group (in nav_order).
    fn is_first_in_file_group(&self, idx: usize, file_key: &str) -> bool {
        for &nav_idx in &self.nav_order {
            if let Some(key) = self.file_key_for_change(nav_idx) {
                if key == file_key {
                    return nav_idx == idx;
                }
            }
        }
        false
    }

    /// Load full file content for display (new/current version), with caching
    pub fn load_file_content(&mut self, rel_path: &std::path::Path) -> Option<&str> {
        if !self.file_cache.contains_key(rel_path) {
            let full_path = if let Some(ref root) = self.repo_root {
                root.join(rel_path)
            } else {
                rel_path.to_path_buf()
            };
            if let Ok(content) = std::fs::read_to_string(&full_path) {
                self.file_cache.insert(rel_path.to_path_buf(), content);
            }
        }
        self.file_cache.get(rel_path).map(|s| s.as_str())
    }

    /// Load old version of a file content for side-by-side view, with caching.
    /// Uses old_root if available, otherwise falls back to repo_root.
    pub fn load_old_file_content(&mut self, rel_path: &std::path::Path) -> Option<&str> {
        // Use a prefixed cache key to distinguish old from new
        let cache_key = PathBuf::from("__old__").join(rel_path);
        if !self.file_cache.contains_key(&cache_key) {
            let full_path = if let Some(ref root) = self.old_root {
                root.join(rel_path)
            } else if let Some(ref root) = self.repo_root {
                root.join(rel_path)
            } else {
                rel_path.to_path_buf()
            };
            if let Ok(content) = std::fs::read_to_string(&full_path) {
                self.file_cache.insert(cache_key.clone(), content);
            }
        }
        self.file_cache.get(&cache_key).map(|s| s.as_str())
    }
}

/// Compare file paths for directory-structure ordering (same as summary panel).
fn cmp_file_paths(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts: Vec<&str> = a.split('/').collect();
    let b_parts: Vec<&str> = b.split('/').collect();
    for (ap, bp) in a_parts.iter().zip(b_parts.iter()) {
        let ac = ap.to_lowercase();
        let bc = bp.to_lowercase();
        match ac.cmp(&bc) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    a_parts.len().cmp(&b_parts.len())
}

/// Build navigation order: change indices grouped by file, matching summary panel display.
fn build_nav_order(diff_result: &DiffResult) -> Vec<usize> {
    let mut file_groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut file_order: HashMap<String, usize> = HashMap::new();

    for (i, change) in diff_result.changes.iter().enumerate() {
        let file = change.file_info();
        if let Some(&idx) = file_order.get(&file) {
            file_groups[idx].1.push(i);
        } else {
            let idx = file_groups.len();
            file_order.insert(file.clone(), idx);
            file_groups.push((file, vec![i]));
        }
    }

    // Sort file groups by directory path (matching summary panel order)
    file_groups.sort_by(|(a, _), (b, _)| cmp_file_paths(a, b));

    // Sort changes within each file by line number (top of file first)
    for (_, indices) in &mut file_groups {
        indices.sort_by_key(|&i| {
            let change = &diff_result.changes[i];
            change
                .new_symbol
                .as_ref()
                .or(change.old_symbol.as_ref())
                .map(|s| s.line_range.0)
                .unwrap_or(0)
        });
    }

    // Flatten: file group order → changes within each group
    file_groups
        .into_iter()
        .flat_map(|(_, indices)| indices)
        .collect()
}

/// Find the first changed (non-Equal) line number within a change's body diff.
/// Always returns old-side line numbers (for consistent scroll positioning).
/// For Added/Deleted, returns the block start. For modifications, walks the
/// body_diff to find the first Delete/Insert line.
fn find_first_changed_line(change: &SemanticChange, block_start: usize) -> usize {
    // For ADD/DEL, the whole block is the change
    if matches!(change.kind, ChangeKind::Added | ChangeKind::Deleted) {
        return block_start;
    }

    let Some(ref diff) = change.body_diff else {
        return block_start;
    };

    let old_start = change
        .old_symbol
        .as_ref()
        .map(|s| s.line_range.0)
        .unwrap_or(block_start);

    let mut old_line = old_start;

    for dl in &diff.lines {
        match dl.tag {
            DiffLineTag::Equal => {
                old_line += 1;
            }
            DiffLineTag::Delete | DiffLineTag::Insert => {
                // Return old-side position for both delete and insert
                return old_line;
            }
        }
    }

    block_start
}
