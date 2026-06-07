use ratatui::{
    Frame,
    layout::Rect,
    style::Stylize,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    client::app::{ChatApp, ChatMessage, MessageState},
    protocol::parse_chat_message,
};

use super::{
    layout::centered_rect,
    theme::{
        body_style, brand_style, client_badge_style, disconnected_border_style,
        disconnected_title_style, fallback_message_style, input_border_style, input_text_style,
        mention_plain_style, mention_selected_style, messages_border_style, muted_style,
        online_user_style, session_border_style, system_message_style, timestamp_style,
        username_style, users_title_style,
    },
};

pub fn render_header(frame: &mut Frame, area: Rect) {
    let header = Paragraph::new(Line::from(vec![
        Span::styled("drocsid", brand_style()),
        Span::raw("  "),
        Span::styled("client", client_badge_style()),
    ]))
    .block(Block::default().borders(Borders::BOTTOM))
    .centered();

    frame.render_widget(header, area);
}

pub fn render_messages_panel(frame: &mut Frame, app: &ChatApp, area: Rect) {
    let messages_text = messages_text(app);
    let chat_block = Block::bordered()
        .title("Messages")
        .border_style(messages_border_style());
    let inner = chat_block.inner(area);
    let scroll = message_scroll_offset(&messages_text, inner);
    let messages = Paragraph::new(messages_text)
        .block(chat_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(messages, area);
}

pub fn render_session_panel(frame: &mut Frame, app: &ChatApp, area: Rect) {
    let sidebar = Paragraph::new(sidebar_text(app))
        .block(
            Block::bordered()
                .title("Session")
                .border_style(session_border_style()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(sidebar, area);
}

pub fn render_input_panel(frame: &mut Frame, app: &ChatApp, area: Rect) {
    let input = Paragraph::new(app.input.as_str())
        .block(
            Block::bordered()
                .title("Input")
                .border_style(input_border_style()),
        )
        .style(input_text_style());

    frame.render_widget(input, area);
}

pub fn render_help_bar(frame: &mut Frame, area: Rect) {
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

    frame.render_widget(help, area);
}

pub fn render_mention_popup(frame: &mut Frame, app: &ChatApp, input_area: Rect) {
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
                mention_selected_style()
            } else {
                mention_plain_style()
            };

            Line::from(vec![Span::styled(format!("@{candidate}"), style)])
        })
        .collect::<Vec<_>>();

    let popup = Paragraph::new(Text::from(lines))
        .block(
            Block::bordered()
                .title("Mention")
                .border_style(input_border_style()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, popup_area);
    frame.render_widget(popup, popup_area);
}

pub fn render_shutdown_popup(frame: &mut Frame, app: &ChatApp) {
    let Some(reason) = &app.exit_notice else {
        return;
    };

    let popup_area = centered_rect(frame.area(), 50, 20);
    let popup = Paragraph::new(Text::from(vec![
        Line::from(Span::styled(
            "Server connection lost",
            disconnected_title_style(),
        )),
        Line::from(""),
        Line::from(reason.clone()),
        Line::from(""),
        Line::from(Span::styled("[ OK ]", mention_selected_style())),
        Line::from(""),
        Line::from(Span::styled("Press Enter to close", muted_style())),
    ]))
    .block(
        Block::bordered()
            .title("Disconnected")
            .border_style(disconnected_border_style()),
    )
    .centered()
    .wrap(Wrap { trim: false });

    frame.render_widget(Clear, popup_area);
    frame.render_widget(popup, popup_area);
}

pub fn set_input_cursor(frame: &mut Frame, app: &ChatApp, area: Rect) {
    let input_area = Block::bordered().inner(area);
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
            .map(|message| chat_line(message, app))
            .collect::<Vec<_>>(),
    )
}

fn chat_line(message: &ChatMessage, app: &ChatApp) -> Line<'static> {
    if message.text.starts_with("[system]") {
        return Line::from(vec![Span::styled(
            message.text.clone(),
            system_message_style(),
        )]);
    }

    if let Some((username, timestamp, body)) = parse_chat_message(&message.text) {
        return chat_message_line(message, app, username, timestamp, body);
    }

    let fallback_style = fallback_line_style(message, app);
    Line::from(vec![Span::styled(message.text.clone(), fallback_style)])
}

fn chat_message_line(
    message: &ChatMessage,
    app: &ChatApp,
    username: &str,
    timestamp: &str,
    body: &str,
) -> Line<'static> {
    let mention_highlight = app.message_mentions_current_user(body);
    let username_style = username_style(message.state == MessageState::Pending);
    let timestamp_style = timestamp_style();
    let body_style = body_style(message.state == MessageState::Pending);

    let (username_style, timestamp_style, colon_style, body_style) = if mention_highlight {
        (
            username_style.bg(super::theme::PASTEL_YELLOW),
            timestamp_style.bg(super::theme::PASTEL_YELLOW),
            body_style
                .fg(ratatui::style::Color::Black)
                .bg(super::theme::PASTEL_YELLOW),
            body_style
                .fg(ratatui::style::Color::Black)
                .bg(super::theme::PASTEL_YELLOW),
        )
    } else {
        (username_style, timestamp_style, body_style, body_style)
    };

    Line::from(vec![
        Span::styled(format!("[{username}]"), username_style),
        Span::styled(format!("({timestamp})"), timestamp_style),
        Span::styled(":", colon_style),
        Span::raw(" "),
        Span::styled(body.to_string(), body_style),
    ])
}

fn fallback_line_style(message: &ChatMessage, app: &ChatApp) -> ratatui::style::Style {
    let fallback_style = fallback_message_style(message.state == MessageState::Pending);
    if app.message_mentions_current_user(&message.text) {
        fallback_style
            .fg(ratatui::style::Color::Black)
            .bg(super::theme::PASTEL_YELLOW)
    } else {
        fallback_style
    }
}

fn sidebar_text(app: &ChatApp) -> Text<'static> {
    let mut lines = vec![Line::from(Span::styled("users", users_title_style()))];

    if app.connected_users.is_empty() {
        lines.push(Line::from(Span::styled(
            "  nobody connected",
            muted_style(),
        )));
    } else {
        for user in &app.connected_users {
            lines.push(Line::from(vec![
                Span::styled("  ", ratatui::style::Style::new()),
                Span::styled(user.clone(), online_user_style()),
            ]));
        }
    }

    Text::from(lines)
}

fn message_scroll_offset(text: &Text<'_>, area: Rect) -> u16 {
    let visible_height = area.height.saturating_sub(1) as usize;
    let total_lines = text.lines.len();
    total_lines.saturating_sub(visible_height) as u16
}
