use clap::{Parser, Subcommand};

fn non_empty_username(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("username cannot be empty".to_string())
    } else {
        Ok(value.to_string())
    }
}

#[derive(Debug, Parser)]
#[command(name = "drocsid", about = "A terminal chat client and server.")]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Start the TCP chat server.
    Server {
        /// Local address where the server accepts connections.
        #[arg(long)]
        listen: Option<String>,
        /// Artificial delay before broadcasting messages, in milliseconds.
        #[arg(long)]
        latency_ms: Option<u64>,
    },
    /// Start the terminal chat client.
    Client {
        /// Username sent during the client handshake.
        #[arg(long, value_parser = non_empty_username)]
        username: String,
        /// Server address to connect to.
        #[arg(long)]
        connect: Option<String>,
    },
}

#[derive(Debug)]
pub enum Command {
    Server {
        listen: Option<String>,
        latency_ms: Option<u64>,
    },
    Client {
        username: String,
        connect: Option<String>,
    },
}

pub fn parse_command(args: impl IntoIterator<Item = String>) -> Result<Command, clap::Error> {
    Cli::try_parse_from(args).map(|cli| match cli.command {
        CliCommand::Server { listen, latency_ms } => Command::Server { listen, latency_ms },
        CliCommand::Client { username, connect } => Command::Client { username, connect },
    })
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_command};

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_server_with_optional_overrides() {
        let command = parse_command(args(&[
            "drocsid",
            "server",
            "--listen",
            "0.0.0.0:9000",
            "--latency-ms",
            "25",
        ]))
        .unwrap();

        assert!(matches!(
            command,
            Command::Server { listen, latency_ms }
                if listen.as_deref() == Some("0.0.0.0:9000") && latency_ms == Some(25)
        ));
    }

    #[test]
    fn parses_server_without_overrides() {
        let command = parse_command(args(&["drocsid", "server"])).unwrap();

        assert!(matches!(
            command,
            Command::Server {
                listen: None,
                latency_ms: None
            }
        ));
    }

    #[test]
    fn parses_client_with_connection_override() {
        let command = parse_command(args(&[
            "drocsid",
            "client",
            "--connect",
            "chat.example.com:7878",
            "--username",
            "alice",
        ]))
        .unwrap();

        assert!(matches!(
            command,
            Command::Client { username, connect }
                if username == "alice" && connect.as_deref() == Some("chat.example.com:7878")
        ));
    }

    #[test]
    fn rejects_client_without_username() {
        assert!(parse_command(args(&["drocsid", "client"])).is_err());
    }

    #[test]
    fn rejects_blank_username() {
        assert!(parse_command(args(&["drocsid", "client", "--username", "   "])).is_err());
    }

    #[test]
    fn rejects_unknown_subcommand() {
        assert!(parse_command(args(&["drocsid", "unknown"])).is_err());
    }
}
