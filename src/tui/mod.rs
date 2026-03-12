pub mod app;
pub mod panels;
pub mod theme;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};

use app::{App, BottomPanel, PanelFocus};

pub fn run_tui(app: &mut App) -> Result<()> {
    let terminal = ratatui::init();
    let result = run_event_loop(terminal, app);
    ratatui::restore();
    result
}

fn run_event_loop(mut terminal: DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
                {
                    app.should_quit = true;
                }

                match key.code {
                    KeyCode::Char('q') => app.should_quit = true,
                    KeyCode::Tab => app.cycle_focus(),
                    KeyCode::Char('v') => app.toggle_bottom(),
                    KeyCode::Char('b') => app.cycle_bottom_panel(),
                    KeyCode::Up | KeyCode::Char('k') => match app.panel_focus {
                        PanelFocus::Summary => app.select_prev(),
                        PanelFocus::Detail => app.scroll_detail_up(),
                        PanelFocus::Review | PanelFocus::Impact => app.scroll_bottom_up(),
                    },
                    KeyCode::Down | KeyCode::Char('j') => match app.panel_focus {
                        PanelFocus::Summary => app.select_next(),
                        PanelFocus::Detail => app.scroll_detail_down(),
                        PanelFocus::Review | PanelFocus::Impact => app.scroll_bottom_down(),
                    },
                    KeyCode::Right | KeyCode::Char('l') => {
                        if app.panel_focus == PanelFocus::Detail {
                            app.scroll_detail_right();
                        }
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        if app.panel_focus == PanelFocus::Detail {
                            app.scroll_detail_left();
                        }
                    }
                    KeyCode::PageUp => {
                        for _ in 0..10 {
                            match app.panel_focus {
                                PanelFocus::Summary => app.select_prev(),
                                PanelFocus::Detail => app.scroll_detail_up(),
                                PanelFocus::Review | PanelFocus::Impact => {
                                    app.scroll_bottom_up()
                                }
                            }
                        }
                    }
                    KeyCode::PageDown => {
                        for _ in 0..10 {
                            match app.panel_focus {
                                PanelFocus::Summary => app.select_next(),
                                PanelFocus::Detail => app.scroll_detail_down(),
                                PanelFocus::Review | PanelFocus::Impact => {
                                    app.scroll_bottom_down()
                                }
                            }
                        }
                    }
                    KeyCode::Home => {
                        // Find first visible item
                        for (pos, &idx) in app.nav_order.iter().enumerate() {
                            if !app.is_collapsed_nav(idx) {
                                app.nav_pos = pos;
                                app.selected_index = idx;
                                app.auto_scroll_detail();
                                break;
                            }
                        }
                    }
                    KeyCode::End => {
                        // Find last visible item
                        for (pos, &idx) in app.nav_order.iter().enumerate().rev() {
                            if !app.is_collapsed_nav(idx) {
                                app.nav_pos = pos;
                                app.selected_index = idx;
                                app.auto_scroll_detail();
                                break;
                            }
                        }
                    }
                    KeyCode::Char('z') | KeyCode::Enter => {
                        if app.panel_focus == PanelFocus::Summary {
                            app.toggle_file_collapse();
                        }
                    }
                    _ => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    let size = f.area();

    let main_layout = Layout::vertical([
        Constraint::Length(2), // Header
        Constraint::Min(1),   // Body
        Constraint::Length(1), // Footer
    ])
    .split(size);

    render_header(f, main_layout[0], app);

    if app.bottom_visible {
        let body = Layout::vertical([
            Constraint::Percentage(65),
            Constraint::Percentage(35),
        ])
        .split(main_layout[1]);

        let top_panels = Layout::horizontal([
            Constraint::Percentage(35),
            Constraint::Percentage(65),
        ])
        .split(body[0]);

        panels::summary::render(f, top_panels[0], app);
        panels::detail::render(f, top_panels[1], app);

        match app.bottom_panel {
            BottomPanel::Review => panels::review::render(f, body[1], app),
            BottomPanel::Impact => panels::impact::render(f, body[1], app),
        }
    } else {
        let body = Layout::horizontal([
            Constraint::Percentage(35),
            Constraint::Percentage(65),
        ])
        .split(main_layout[1]);

        panels::summary::render(f, body[0], app);
        panels::detail::render(f, body[1], app);
    }

    render_footer(f, main_layout[2], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let summary = &app.diff_result.summary;
    let mut spans = vec![
        Span::styled(
            " semdiff ",
            Style::default().fg(Color::Cyan).bg(Color::DarkGray),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} changes", summary.total_changes),
            Style::default().fg(Color::White),
        ),
        Span::raw("  "),
        Span::styled(format!("+{}", summary.added), Style::default().fg(Color::Green)),
        Span::raw(" "),
        Span::styled(format!("-{}", summary.deleted), Style::default().fg(Color::Red)),
        Span::raw(" "),
        Span::styled(
            format!("~{}", summary.modified + summary.signature_changed),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" "),
        Span::styled(format!(">{}", summary.moved), Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(
            format!("^{}", summary.extracted),
            Style::default().fg(Color::Magenta),
        ),
    ];

    if let Some(ref analysis) = app.repo_analysis {
        spans.push(Span::raw("  |  "));
        spans.push(Span::styled(
            format!(
                "impact: {} callers, {} similar",
                analysis.impact.affected_callers.len(),
                analysis.impact.similar_code.len(),
            ),
            Style::default().fg(Color::Yellow),
        ));
    }

    let header = Line::from(spans);
    let block = Block::default().borders(Borders::BOTTOM);
    let paragraph = Paragraph::new(header).block(block);
    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let status = if let Some(ref msg) = app.status_message {
        Span::styled(msg.clone(), Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };

    let bottom_label = match app.bottom_panel {
        BottomPanel::Review => "review",
        BottomPanel::Impact => "impact",
    };

    let footer = Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Cyan)),
        Span::raw(":quit "),
        Span::styled("Tab", Style::default().fg(Color::Cyan)),
        Span::raw(":focus "),
        Span::styled("j/k", Style::default().fg(Color::Cyan)),
        Span::raw(":nav "),
        Span::styled("h/l", Style::default().fg(Color::Cyan)),
        Span::raw(":scroll "),
        Span::styled("v", Style::default().fg(Color::Cyan)),
        Span::raw(format!(":{} ", bottom_label)),
        Span::styled("b", Style::default().fg(Color::Cyan)),
        Span::raw(":switch "),
        Span::styled("z", Style::default().fg(Color::Cyan)),
        Span::raw(":fold "),
        status,
    ]);

    let paragraph = Paragraph::new(footer);
    f.render_widget(paragraph, area);
}
