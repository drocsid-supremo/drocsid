use std::{
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::Duration,
};

use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{config, error::AppError};

const PASTEL_YELLOW: Color = Color::Rgb(245, 229, 168);
const PASTEL_YELLOW_BORDER: Color = Color::Rgb(226, 208, 140);

enum NetworkEvent {
    Message(String),
    UserList(Vec<String>),
    Disconnected(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageState {
    Pending,
    Confirmed,
}

struct ChatMessage {
    text: String,
    state: MessageState,
}

struct ChatApp {
    username: String,
    input: String,
    messages: Vec<ChatMessage>,
    connected_users: Vec<String>,
    mention_selection: usize,
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
                ChatMessage {
                    text: format!("[system] Connected to {server_addr}"),
                    state: MessageState::Confirmed,
                },
                ChatMessage {
                    text: "[system] Type a message and press Enter.".to_string(),
                    state: MessageState::Confirmed,
                },
            ],
            connected_users: vec![username.to_string()],
            mention_selection: 0,
            status: "online".to_string(),
            connected: true,
            should_quit: false,
        }
    }

    fn push_message(&mut self, message: impl Into<String>, state: MessageState) {
        self.messages.push(ChatMessage {
            text: message.into(),
            state,
        });

        if self.messages.len() > 300 {
            let overflow = self.messages.len() - 300;
            self.messages.drain(0..overflow);
        }
    }

    fn confirm_message(&mut self, message: &str) -> bool {
        if let Some(entry) = self
            .messages
            .iter_mut()
            .find(|entry| entry.text == message && entry.state == MessageState::Pending)
        {
            entry.state = MessageState::Confirmed;
            return true;
        }

        false
    }

    fn mention_candidates(&self) -> Vec<String> {
        let Some(query) = active_mention_query(&self.input) else {
            return Vec::new();
        };

        let query_lower = query.to_ascii_lowercase();
        self.connected_users
            .iter()
            .filter(|user| user.to_ascii_lowercase().starts_with(&query_lower))
            .cloned()
            .collect()
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
                    if let Some(users) = parse_users_event(&line) {
                        let _ = tx.send(NetworkEvent::UserList(users));
                    } else {
                        let _ = tx.send(NetworkEvent::Message(line));
                    }
                }
            }
        }
    });
}

fn drain_network_events(app: &mut ChatApp, rx: &Receiver<NetworkEvent>) {
    loop {
        match rx.try_recv() {
            Ok(NetworkEvent::Message(message)) => {
                if !app.confirm_message(&message) {
                    app.push_message(message, MessageState::Confirmed);
                }
            }
            Ok(NetworkEvent::UserList(users)) => {
                app.connected_users = users;
                let candidates_len = app.mention_candidates().len();
                if candidates_len == 0 {
                    app.mention_selection = 0;
                } else if app.mention_selection >= candidates_len {
                    app.mention_selection = candidates_len - 1;
                }
            }
            Ok(NetworkEvent::Disconnected(reason)) => {
                app.connected = false;
                app.status = reason.clone();
                app.push_message(format!("[system] {reason}"), MessageState::Confirmed);
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
            KeyCode::Up => select_previous_mention(app),
            KeyCode::Down => select_next_mention(app),
            KeyCode::Tab => apply_selected_mention(app),
            KeyCode::Enter => submit_input(app, stream)?,
            KeyCode::Backspace => {
                app.input.pop();
                reset_mention_selection(app);
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.input.push(character);
                reset_mention_selection(app);
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
        app.push_message(
            "[system] message not sent because the client is offline",
            MessageState::Confirmed,
        );
        return Ok(());
    }

    let current_time = Local::now().format("%H:%M");
    let formatted = format!("[{}]({}): {}", app.username, current_time, content);
    let wire_message = format!("{formatted}\n");

    if let Err(error) = stream.write_all(wire_message.as_bytes()) {
        app.connected = false;
        app.status = "offline".to_string();
        app.push_message(
            format!("[system] failed to send message: {error}"),
            MessageState::Confirmed,
        );

        if matches!(
            error.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
        ) {
            return Ok(());
        }

        return Err(error);
    }

    app.push_message(formatted, MessageState::Pending);
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
            "drocsid",
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
        .border_style(Style::new().fg(PASTEL_YELLOW_BORDER));
    let input_area = input_block.inner(layout[2]);
    let input = Paragraph::new(app.input.as_str())
        .block(input_block)
        .style(Style::new().fg(Color::White));

    let help = Paragraph::new(Line::from(vec![
        "Enter".bold().yellow(),
        " send  ".into(),
        "Tab".bold().yellow(),
        " mention  ".into(),
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
    render_mention_popup(frame, app, input_area);

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
            .map(|message| chat_line(message, &app.username))
            .collect::<Vec<_>>(),
    )
}

fn chat_line(message: &ChatMessage, current_username: &str) -> Line<'static> {
    if message.text.starts_with("[system]") {
        return Line::from(vec![Span::styled(
            message.text.clone(),
            Style::new().fg(Color::Magenta),
        )]);
    }

    if let Some((username, timestamp, body)) = parse_chat_message(&message.text) {
        let mention_highlight = message_mentions_user(body, current_username);
        let username_style = if message.state == MessageState::Pending {
            Style::new().fg(Color::Gray)
        } else {
            Style::new().fg(Color::Cyan)
        };
        let timestamp_style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM);
        let body_style = if message.state == MessageState::Pending {
            Style::new().fg(Color::Gray)
        } else {
            Style::new().fg(Color::White)
        };

        let (username_style, timestamp_style, colon_style, body_style) = if mention_highlight {
            (
                username_style.bg(PASTEL_YELLOW),
                timestamp_style.bg(PASTEL_YELLOW),
                body_style.fg(Color::Black).bg(PASTEL_YELLOW),
                body_style.fg(Color::Black).bg(PASTEL_YELLOW),
            )
        } else {
            (username_style, timestamp_style, body_style, body_style)
        };

        return Line::from(vec![
            Span::styled(format!("[{username}]"), username_style),
            Span::styled(format!("({timestamp})"), timestamp_style),
            Span::styled(":", colon_style),
            Span::raw(" "),
            Span::styled(body.to_string(), body_style),
        ]);
    }

    let fallback_style = if message.state == MessageState::Pending {
        Style::new().fg(Color::Gray)
    } else {
        Style::new()
    };
    let fallback_style = if message_mentions_user(&message.text, current_username) {
        fallback_style.fg(Color::Black).bg(PASTEL_YELLOW)
    } else {
        fallback_style
    };

    Line::from(vec![Span::styled(message.text.clone(), fallback_style)])
}

fn parse_chat_message(message: &str) -> Option<(&str, &str, &str)> {
    let close_user = message.find("](")?;
    let close_time = message[close_user + 2..].find("): ")? + close_user + 2;
    let username = message.get(1..close_user)?;
    let timestamp = message.get(close_user + 2..close_time)?;
    let body = message.get(close_time + 3..)?;

    if message.starts_with('[') {
        Some((username, timestamp, body))
    } else {
        None
    }
}

fn sidebar_text(app: &ChatApp) -> Text<'static> {
    let status_style = if app.connected {
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let mut lines = vec![
        Line::from(vec!["User: ".into(), app.username.clone().bold()]),
        Line::from(vec![
            "Status: ".into(),
            Span::styled(app.status.clone(), status_style),
        ]),
        Line::from(vec![
            "Messages: ".into(),
            app.messages.len().to_string().yellow(),
        ]),
        Line::from(vec![
            "Online: ".into(),
            app.connected_users.len().to_string().yellow(),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "users",
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        )),
    ];

    if app.connected_users.is_empty() {
        lines.push(Line::from(Span::styled(
            "  nobody connected",
            Style::new().fg(Color::DarkGray),
        )));
    } else {
        for user in &app.connected_users {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(user.clone(), Style::new().fg(Color::Cyan)),
            ]));
        }
    }

    Text::from(lines)
}

fn render_mention_popup(frame: &mut Frame, app: &ChatApp, input_area: Rect) {
    let candidates = app.mention_candidates();
    if candidates.is_empty() {
        return;
    }

    let height = candidates.len().min(5) as u16 + 2;
    let width = candidates
        .iter()
        .map(|candidate| candidate.len() as u16 + 3)
        .max()
        .unwrap_or(12)
        .max(18);
    let popup_area = Rect {
        x: input_area.x,
        y: input_area.y.saturating_sub(height + 1),
        width: width.min(frame.area().width.saturating_sub(input_area.x)),
        height,
    };

    let lines = candidates
        .iter()
        .enumerate()
        .take(5)
        .map(|(index, candidate)| {
            let style = if index == app.mention_selection {
                Style::new().fg(Color::Black).bg(PASTEL_YELLOW)
            } else {
                Style::new().fg(Color::White)
            };

            Line::from(vec![Span::styled(format!("@{candidate}"), style)])
        })
        .collect::<Vec<_>>();

    let popup = Paragraph::new(Text::from(lines))
        .block(
            Block::bordered()
                .title("Mention")
                .border_style(Style::new().fg(PASTEL_YELLOW_BORDER)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, popup_area);
    frame.render_widget(popup, popup_area);
}

fn message_scroll_offset(text: &Text<'_>, area: Rect) -> u16 {
    let visible_height = area.height.saturating_sub(1) as usize;
    let total_lines = text.lines.len();

    total_lines.saturating_sub(visible_height) as u16
}

fn parse_users_event(line: &str) -> Option<Vec<String>> {
    let payload = line.strip_prefix("__users__:")?;
    if payload.is_empty() {
        return Some(Vec::new());
    }

    Some(
        payload
            .split(',')
            .map(str::trim)
            .filter(|user| !user.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn active_mention_query(input: &str) -> Option<&str> {
    let at_index = input.rfind('@')?;
    let mention = input.get(at_index + 1..)?;

    if mention.contains(char::is_whitespace) {
        return None;
    }

    if at_index > 0 {
        let previous = input[..at_index].chars().last()?;
        if !previous.is_whitespace() {
            return None;
        }
    }

    Some(mention)
}

fn select_previous_mention(app: &mut ChatApp) {
    let candidates = app.mention_candidates();
    if candidates.is_empty() {
        return;
    }

    if app.mention_selection == 0 {
        app.mention_selection = candidates.len() - 1;
    } else {
        app.mention_selection -= 1;
    }
}

fn select_next_mention(app: &mut ChatApp) {
    let candidates = app.mention_candidates();
    if candidates.is_empty() {
        return;
    }

    app.mention_selection = (app.mention_selection + 1) % candidates.len();
}

fn apply_selected_mention(app: &mut ChatApp) {
    let candidates = app.mention_candidates();
    if candidates.is_empty() {
        return;
    }

    let mention = &candidates[app.mention_selection];
    let Some(at_index) = app.input.rfind('@') else {
        return;
    };

    app.input.truncate(at_index);
    app.input.push('@');
    app.input.push_str(mention);
    app.input.push(' ');
    app.mention_selection = 0;
}

fn reset_mention_selection(app: &mut ChatApp) {
    app.mention_selection = 0;
}

fn message_mentions_user(body: &str, username: &str) -> bool {
    let needle = format!("@{username}");
    let mut search_start = 0;

    while let Some(relative_index) = body[search_start..].find(&needle) {
        let index = search_start + relative_index;
        let after = body[index + needle.len()..].chars().next();

        if after.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_') {
            return true;
        }

        search_start = index + needle.len();
    }

    false
}
