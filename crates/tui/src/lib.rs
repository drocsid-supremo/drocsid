mod components;
mod layout;
mod theme;

use std::{
    io,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use drocsid_client::{
    app::{ChatApp, MessageState},
    network::{ClientConnection, NetworkEvent},
};
use drocsid_config::ServerConfig;
use drocsid_protocol::format_chat_message;
use ratatui::Frame;
use tracing::info;

pub fn run_client(username: &str, server_config: &ServerConfig) -> io::Result<()> {
    if username.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "username cannot be empty",
        ));
    }

    let (mut connection, rx) = ClientConnection::connect(username, server_config)?;
    let mut app = ChatApp::new(username, &server_config.connect_addr);

    ratatui::run(|terminal| -> io::Result<()> {
        while !app.should_quit {
            drain_network_events(&mut app, &rx);
            terminal.draw(|frame| render(frame, &app))?;
            handle_terminal_event(&mut app, &mut connection)?;
        }

        Ok(())
    })?;

    if let Some(exit_notice) = app.exit_notice {
        info!(reason = %exit_notice, "client closed");
    }

    Ok(())
}

fn drain_network_events(app: &mut ChatApp, rx: &Receiver<NetworkEvent>) {
    loop {
        match rx.try_recv() {
            Ok(NetworkEvent::Message(message)) => app.receive_message(message),
            Ok(NetworkEvent::UserList(users)) => app.update_connected_users(users),
            Ok(NetworkEvent::Disconnected(reason)) => app.begin_shutdown(reason),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }
}

pub fn handle_terminal_event(
    app: &mut ChatApp,
    connection: &mut ClientConnection,
) -> std::io::Result<()> {
    if !event::poll(Duration::from_millis(50))? {
        return Ok(());
    }

    match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            handle_key_event(app, connection, key)
        }
        Event::Resize(_, _) => Ok(()),
        _ => Ok(()),
    }
}

pub fn render(frame: &mut Frame, app: &ChatApp) {
    let app_layout = layout::AppLayout::new(frame.area());

    frame.render_widget(ratatui::widgets::Clear, frame.area());
    components::render_header(frame, app_layout.header);
    components::render_messages_panel(frame, app, app_layout.messages);
    components::render_session_panel(frame, app, app_layout.sidebar);
    components::render_input_panel(frame, app, app_layout.input);
    components::render_help_bar(frame, app_layout.footer);
    components::render_mention_popup(frame, app, app_layout.input);
    components::render_shutdown_popup(frame, app);
    components::set_input_cursor(frame, app, app_layout.input);
}

fn handle_key_event(
    app: &mut ChatApp,
    connection: &mut ClientConnection,
    key: crossterm::event::KeyEvent,
) -> std::io::Result<()> {
    if app.has_exit_notice() {
        if key.code == KeyCode::Enter {
            app.should_quit = true;
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true;
        }
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Up => app.select_previous_mention(),
        KeyCode::Down => app.select_next_mention(),
        KeyCode::Tab => app.apply_selected_mention(),
        KeyCode::Enter => submit_input(app, connection)?,
        KeyCode::Backspace => app.pop_input(),
        KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.append_input(character);
        }
        _ => {}
    }

    Ok(())
}

fn submit_input(app: &mut ChatApp, connection: &mut ClientConnection) -> std::io::Result<()> {
    let content = app.take_input();
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

    let current_time = Local::now().format("%H:%M").to_string();
    let formatted = format_chat_message(&app.username, &current_time, &content);
    let wire_message = format!("{formatted}\n");

    if let Err(error) = connection.send_message(&wire_message) {
        if ClientConnection::is_disconnect_error(&error) {
            app.begin_shutdown(format!("server unavailable: {error}"));
            return Ok(());
        }

        app.mark_offline(format!("failed to send message: {error}"));
        return Err(error);
    }

    app.push_message(formatted, MessageState::Pending);
    Ok(())
}
