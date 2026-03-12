use crate::diff::change::{DiffResult, SemanticChange};
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

        // Auto-scroll to first change's location for side-by-side view
        let initial_scroll = diff_result
            .changes
            .get(selected_index)
            .and_then(|c| c.new_symbol.as_ref().or(c.old_symbol.as_ref()))
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
        }
    }

    pub fn selected_change(&self) -> Option<&SemanticChange> {
        self.diff_result.changes.get(self.selected_index)
    }

    pub fn change_count(&self) -> usize {
        self.diff_result.changes.len()
    }

    pub fn select_next(&mut self) {
        if self.nav_pos + 1 < self.nav_order.len() {
            self.nav_pos += 1;
            self.selected_index = self.nav_order[self.nav_pos];
            self.auto_scroll_detail();
            self.bottom_scroll = 0;
        }
    }

    pub fn select_prev(&mut self) {
        if self.nav_pos > 0 {
            self.nav_pos -= 1;
            self.selected_index = self.nav_order[self.nav_pos];
            self.auto_scroll_detail();
            self.bottom_scroll = 0;
        }
    }

    /// Auto-scroll detail panel to the selected change's location.
    pub fn auto_scroll_detail(&mut self) {
        if let Some(change) = self.diff_result.changes.get(self.selected_index) {
            let sym = change.new_symbol.as_ref().or(change.old_symbol.as_ref());
            if let Some(sym) = sym {
                self.detail_scroll = sym.line_range.0.saturating_sub(3);
                self.detail_h_scroll = 0;
                return;
            }
        }
        self.detail_scroll = 0;
        self.detail_h_scroll = 0;
    }

    pub fn scroll_detail_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(1);
    }

    pub fn scroll_detail_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
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

    // Flatten: file group order → changes within each group
    file_groups
        .into_iter()
        .flat_map(|(_, indices)| indices)
        .collect()
}
