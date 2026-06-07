mod app;
mod network;
mod ui;

use std::sync::mpsc::{Receiver, TryRecvError};

use app::ChatApp;
use network::NetworkEvent;

use crate::{config::ServerConfig, error::AppError};

pub fn run_client(username: &str, server_config: &ServerConfig) -> Result<(), AppError> {
    if username.trim().is_empty() {
        return Err(AppError::MissingUsername);
    }

    let (mut connection, rx) = network::ClientConnection::connect(username, server_config)?;
    let mut app = ChatApp::new(username, &server_config.connect_addr);

    ratatui::run(|terminal| -> std::io::Result<()> {
        while !app.should_quit {
            drain_network_events(&mut app, &rx);
            terminal.draw(|frame| ui::render(frame, &app))?;
            ui::handle_terminal_event(&mut app, &mut connection)?;
        }

        Ok(())
    })?;

    if let Some(exit_notice) = app.exit_notice {
        println!("client closed: {exit_notice}");
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
