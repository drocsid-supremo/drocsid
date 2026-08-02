use std::{
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpStream},
    thread,
    time::Duration,
};

use crate::ServerError;

use super::state::{
    ServerStateHandle, broadcast, broadcast_presence, message_history, record_message,
    remove_client, set_client_username,
};

pub struct ConnectionHandler {
    state: ServerStateHandle,
    simulated_latency: Duration,
}

impl ConnectionHandler {
    pub fn new(state: ServerStateHandle, simulated_latency: Duration) -> Self {
        Self {
            state,
            simulated_latency,
        }
    }

    pub fn serve(&self, mut stream: TcpStream) -> Result<(), ServerError> {
        let sender_addr = stream.peer_addr()?;
        let username = self.read_handshake_username(&mut stream, sender_addr)?;

        set_client_username(&self.state, sender_addr, &username)?;
        self.send_message_history(&mut stream)?;
        broadcast_presence(&self.state)?;

        let join_message = format!("@{} has entered the chat. Say hello!\n", username);
        println!("{}", join_message.trim());
        record_message(&self.state, &join_message)?;
        broadcast(&self.state, &join_message, None)?;

        self.read_messages(stream, sender_addr, &username)
    }

    fn read_handshake_username(
        &self,
        stream: &mut TcpStream,
        sender_addr: SocketAddr,
    ) -> Result<String, ServerError> {
        let mut buffer = [0; 1024];
        let bytes = stream.read(&mut buffer)?;

        if bytes == 0 {
            remove_client(&self.state, sender_addr)?;
            return Err(ServerError::EmptyHandshakeUsername);
        }

        let username = String::from_utf8_lossy(&buffer[..bytes]).trim().to_string();
        if username.is_empty() {
            remove_client(&self.state, sender_addr)?;
            return Err(ServerError::EmptyHandshakeUsername);
        }

        Ok(username)
    }

    fn read_messages(
        &self,
        mut stream: TcpStream,
        sender_addr: SocketAddr,
        username: &str,
    ) -> Result<(), ServerError> {
        let mut buffer = [0; 1024];

        loop {
            let bytes = match stream.read(&mut buffer) {
                Ok(bytes) => bytes,
                Err(error) if is_disconnect_error(&error) => {
                    self.disconnect_client(sender_addr, username)?;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };

            if bytes == 0 {
                self.disconnect_client(sender_addr, username)?;
                return Ok(());
            }

            let message = String::from_utf8_lossy(&buffer[..bytes]);

            if !self.simulated_latency.is_zero() {
                thread::sleep(self.simulated_latency);
            }

            record_message(&self.state, &message)?;
            broadcast(&self.state, &message, None)?;
        }
    }

    fn disconnect_client(
        &self,
        sender_addr: SocketAddr,
        username: &str,
    ) -> Result<(), ServerError> {
        remove_client(&self.state, sender_addr)?;

        let leave_message = format!("{username} has left the chat\n");
        println!("{}", leave_message.trim());
        record_message(&self.state, &leave_message)?;
        broadcast(&self.state, &leave_message, None)?;
        broadcast_presence(&self.state)?;

        Ok(())
    }

    fn send_message_history(&self, stream: &mut TcpStream) -> Result<(), ServerError> {
        for message in message_history(&self.state)? {
            stream.write_all(message.as_bytes())?;
            stream.write_all(b"\n")?;
        }

        Ok(())
    }
}

fn is_disconnect_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::BrokenPipe | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
    )
}
