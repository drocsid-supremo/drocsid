use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::error::AppError;

#[derive(Default)]
struct App {
    should_quit: bool,
}

pub fn run_tui() -> Result<(), AppError> {
    ratatui::run(|terminal| -> std::io::Result<()> {
        let mut app = App::default();

        while !app.should_quit {
            terminal.draw(|frame| render(frame, &app))?;
            handle_events(&mut app)?;
        }

        Ok(())
    })?;

    Ok(())
}

fn render(frame: &mut Frame, app: &App) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Length(3),
    ])
    .split(frame.area());
    let columns = Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(areas[1]);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "drocsid tui",
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("prototype", Style::new().fg(Color::Yellow)),
    ]))
    .block(Block::default().borders(Borders::BOTTOM))
    .centered();

    let messages = Paragraph::new(chat_text())
        .block(
            Block::bordered()
                .title("Chat")
                .border_style(Style::new().fg(Color::Blue)),
        )
        .wrap(Wrap { trim: false });

    let sidebar = Paragraph::new(sidebar_text())
        .block(
            Block::bordered()
                .title("Status")
                .border_style(Style::new().fg(Color::Green)),
        )
        .wrap(Wrap { trim: false });

    let footer = Paragraph::new(Line::from(vec![
        "Pressione ".into(),
        "q".bold().yellow(),
        " para sair, ".into(),
        "r".bold().yellow(),
        " para redesenhar.".into(),
    ]))
    .block(Block::default().borders(Borders::TOP));

    frame.render_widget(Clear, frame.area());
    frame.render_widget(header, areas[0]);
    frame.render_widget(messages, columns[0]);
    frame.render_widget(sidebar, columns[1]);
    frame.render_widget(footer, areas[2]);

    if app.should_quit {
        let popup = centered_rect(frame.area(), 40, 20);
        let dialog = Paragraph::new("Encerrando TUI...")
            .block(Block::bordered().title("Bye"))
            .centered();
        frame.render_widget(Clear, popup);
        frame.render_widget(dialog, popup);
    }
}

fn handle_events(app: &mut App) -> std::io::Result<()> {
    if event::poll(Duration::from_millis(250))? {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                KeyCode::Char('r') => {}
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }

    Ok(())
}

fn chat_text() -> Text<'static> {
    Text::from(vec![
        Line::from("Welcome to the Ratatui prototype."),
        Line::from(""),
        Line::from(vec![
            "[system]".fg(Color::Magenta),
            " This screen is only a UI scaffold for now.".into(),
        ]),
        Line::from(vec![
            "[next]".fg(Color::Cyan),
            " Wire this to the TCP client and render live messages.".into(),
        ]),
        Line::from(""),
        Line::from("Suggested next steps:"),
        Line::from("- keep messages in app state"),
        Line::from("- capture typed input in a footer box"),
        Line::from("- connect network events through channels"),
    ])
}

fn sidebar_text() -> Text<'static> {
    Text::from(vec![
        Line::from(vec!["Mode: ".into(), "TUI".bold().green()]),
        Line::from(vec!["Backend: ".into(), "ratatui + crossterm".into()]),
        Line::from(""),
        Line::from("Keybinds"),
        Line::from("- q / Esc: quit"),
        Line::from("- r: redraw"),
    ])
}

fn centered_rect(
    area: ratatui::layout::Rect,
    width_percent: u16,
    height_percent: u16,
) -> ratatui::layout::Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_percent) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage((100 - height_percent) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - width_percent) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage((100 - width_percent) / 2),
    ])
    .split(vertical[1])[1]
}
