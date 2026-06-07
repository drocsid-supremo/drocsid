use std::{
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::Duration,
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{config, error::AppError};

enum NetworkEvent {
    Message(String),
    Disconnected(String),
}

struct ChatApp {
    username: String,
    input: String,
    messages: Vec<String>,
    status: String,
    connected: bool,
    should_quit: bool,
}

impl ChatApp {
    fn new(username: &str) -> Self {
        let server_addr = config::server_connect_addr();

        Self {
            username: username.to_string(),
            input: String::new(),
            messages: vec![
                format!("Connected to {server_addr}"),
                "Type a message and press Enter.".to_string(),
            ],
            status: "online".to_string(),
            connected: true,
            should_quit: false,
        }
    }

    fn push_message(&mut self, message: impl Into<String>) {
        self.messages.push(message.into());

        if self.messages.len() > 300 {
            let overflow = self.messages.len() - 300;
            self.messages.drain(0..overflow);
        }
    }
}

pub fn run_client(username: &str) -> Result<(), AppError> {
    if username.trim().is_empty() {
        return Err(AppError::MissingUsername);
    }

    let server_addr = config::server_connect_addr();
    let mut stream = TcpStream::connect(&server_addr)?;
    let reader_stream = stream.try_clone()?;

    let join_msg = format!("{username}\n");
    stream.write_all(join_msg.as_bytes())?;

    let (tx, rx) = mpsc::channel();
    spawn_reader(reader_stream, tx);

    ratatui::run(|terminal| -> std::io::Result<()> {
        let mut app = ChatApp::new(username);

        while !app.should_quit {
            drain_network_events(&mut app, &rx);
            terminal.draw(|frame| render(frame, &app))?;
            handle_input(&mut app, &mut stream)?;
        }

        Ok(())
    })?;

    Ok(())
}

fn spawn_reader(mut reader_stream: TcpStream, tx: Sender<NetworkEvent>) {
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        let mut pending = String::new();

        loop {
            let bytes = match reader_stream.read(&mut buffer) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let _ = tx.send(NetworkEvent::Disconnected(format!(
                        "connection error: {error}"
                    )));
                    break;
                }
            };

            if bytes == 0 {
                if !pending.trim().is_empty() {
                    let _ = tx.send(NetworkEvent::Message(
                        pending.trim_end_matches('\n').to_string(),
                    ));
                }

                let _ = tx.send(NetworkEvent::Disconnected(
                    "server disconnected".to_string(),
                ));
                break;
            }

            pending.push_str(&String::from_utf8_lossy(&buffer[..bytes]));

            while let Some(newline_index) = pending.find('\n') {
                let line = pending[..newline_index].to_string();
                pending.drain(..=newline_index);

                if !line.trim().is_empty() {
                    let _ = tx.send(NetworkEvent::Message(line));
                }
            }
        }
    });
}

fn drain_network_events(app: &mut ChatApp, rx: &Receiver<NetworkEvent>) {
    loop {
        match rx.try_recv() {
            Ok(NetworkEvent::Message(message)) => app.push_message(message),
            Ok(NetworkEvent::Disconnected(reason)) => {
                app.connected = false;
                app.status = reason.clone();
                app.push_message(format!("[system] {reason}"));
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }
}

fn handle_input(app: &mut ChatApp, stream: &mut TcpStream) -> std::io::Result<()> {
    if !event::poll(Duration::from_millis(50))? {
        return Ok(());
    }

    match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.should_quit = true;
            }
            KeyCode::Esc => app.should_quit = true,
            KeyCode::Enter => submit_input(app, stream)?,
            KeyCode::Backspace => {
                app.input.pop();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.input.push(character);
            }
            _ => {}
        },
        Event::Resize(_, _) => {}
        _ => {}
    }

    Ok(())
}

fn submit_input(app: &mut ChatApp, stream: &mut TcpStream) -> std::io::Result<()> {
    let content = app.input.trim().to_string();
    app.input.clear();

    if content.is_empty() {
        return Ok(());
    }

    if !app.connected {
        app.push_message("[system] message not sent because the client is offline");
        return Ok(());
    }

    let formatted = format!("[{}]: {}", app.username, content);
    let wire_message = format!("{formatted}\n");

    if let Err(error) = stream.write_all(wire_message.as_bytes()) {
        app.connected = false;
        app.status = "offline".to_string();
        app.push_message(format!("[system] failed to send message: {error}"));

        if matches!(
            error.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
        ) {
            return Ok(());
        }

        return Err(error);
    }

    app.push_message(formatted);
    Ok(())
}

fn render(frame: &mut Frame, app: &ChatApp) {
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(frame.area());

    let body = Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
        .split(layout[1]);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "discordia",
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("client", Style::new().fg(Color::Black).bg(Color::Green)),
    ]))
    .block(Block::default().borders(Borders::BOTTOM))
    .centered();

    let messages_text = messages_text(app);
    let chat_block = Block::bordered()
        .title("Messages")
        .border_style(Style::new().fg(Color::Blue));
    let inner = chat_block.inner(body[0]);
    let scroll = message_scroll_offset(&messages_text, inner);
    let messages = Paragraph::new(messages_text)
        .block(chat_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    let sidebar = Paragraph::new(sidebar_text(app))
        .block(
            Block::bordered()
                .title("Session")
                .border_style(Style::new().fg(Color::Green)),
        )
        .wrap(Wrap { trim: false });

    let input_block = Block::bordered()
        .title("Input")
        .border_style(Style::new().fg(Color::Yellow));
    let input_area = input_block.inner(layout[2]);
    let input = Paragraph::new(app.input.as_str())
        .block(input_block)
        .style(Style::new().fg(Color::White));

    let help = Paragraph::new(Line::from(vec![
        "Enter".bold().yellow(),
        " send  ".into(),
        "Esc".bold().yellow(),
        " quit  ".into(),
        "Ctrl+Q".bold().yellow(),
        " force quit".into(),
    ]))
    .block(Block::default().borders(Borders::TOP));

    frame.render_widget(Clear, frame.area());
    frame.render_widget(header, layout[0]);
    frame.render_widget(messages, body[0]);
    frame.render_widget(sidebar, body[1]);
    frame.render_widget(input, layout[2]);
    frame.render_widget(help, layout[3]);

    let cursor_x = input_area
        .x
        .saturating_add(app.input.chars().count() as u16);
    let max_cursor_x = input_area
        .x
        .saturating_add(input_area.width.saturating_sub(1));
    frame.set_cursor_position((cursor_x.min(max_cursor_x), input_area.y));
}

fn messages_text(app: &ChatApp) -> Text<'static> {
    Text::from(
        app.messages
            .iter()
            .map(|message| {
                if message.starts_with("[system]") {
                    Line::from(vec![Span::styled(
                        message.clone(),
                        Style::new().fg(Color::Magenta),
                    )])
                } else if message.starts_with('[') {
                    Line::from(vec![Span::styled(
                        message.clone(),
                        Style::new().fg(Color::Cyan),
                    )])
                } else {
                    Line::from(message.clone())
                }
            })
            .collect::<Vec<_>>(),
    )
}

fn sidebar_text(app: &ChatApp) -> Text<'static> {
    let status_style = if app.connected {
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
    };

    Text::from(vec![
        Line::from(vec!["User: ".into(), app.username.clone().bold()]),
        Line::from(vec![
            "Status: ".into(),
            Span::styled(app.status.clone(), status_style),
        ]),
        Line::from(vec![
            "Messages: ".into(),
            app.messages.len().to_string().yellow(),
        ]),
        Line::from(""),
        Line::from("Notes"),
        Line::from("- local echo is enabled"),
        Line::from("- server messages stream live"),
        Line::from("- offline mode blocks sends"),
    ])
}

fn message_scroll_offset(text: &Text<'_>, area: Rect) -> u16 {
    let visible_height = area.height.saturating_sub(1) as usize;
    let total_lines = text.lines.len();

    total_lines.saturating_sub(visible_height) as u16
}
