use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::diff::change::ChangeKind;
use crate::tui::app::{App, PanelFocus};
use crate::tui::theme;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let border_style = if app.panel_focus == PanelFocus::Detail {
        theme::focus_border_style()
    } else {
        theme::normal_border_style()
    };

    let title = if let Some(change) = app.selected_change() {
        match &change.kind {
            ChangeKind::Added => " New File ".to_string(),
            ChangeKind::Deleted => " Deleted File ".to_string(),
            _ => " Side-by-Side Diff ".to_string(),
        }
    } else {
        " Diff ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.selected_change().is_none() {
        let empty = Paragraph::new("No change selected").block(block);
        f.render_widget(empty, area);
        return;
    }

    let inner = block.inner(area);
    f.render_widget(block, area);

    super::sidebyside::render_inline(f, inner, app);
}
