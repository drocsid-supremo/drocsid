use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("missing mode argument, expected `mode=server` or `mode=client`")]
    MissingMode,
    #[error("missing username argument, expected `username=<name>`")]
    MissingUsername,
    #[error("invalid mode `{0}`, expected `server` or `client`")]
    InvalidMode(String),
}

#[derive(Debug)]
pub enum Command {
    Server,
    Client { username: String },
}

pub fn parse_command(args: impl IntoIterator<Item = String>) -> Result<Command, CliError> {
    let mut mode = None;
    let mut username = None;

    for arg in args {
        if let Some(value) = arg.strip_prefix("mode=") {
            mode = Some(value.to_owned());
        }

        if let Some(value) = arg.strip_prefix("username=") {
            username = Some(value.to_owned());
        }
    }

    match mode.as_deref().ok_or(CliError::MissingMode)? {
        "server" => Ok(Command::Server),
        "client" => {
            let username = username
                .filter(|value| !value.trim().is_empty())
                .ok_or(CliError::MissingUsername)?;

            Ok(Command::Client { username })
        }
        other => Err(CliError::InvalidMode(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::CliError;
    use super::{Command, parse_command};

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_server_mode() {
        let command = parse_command(args(&["bin", "mode=server"])).unwrap();

        assert!(matches!(command, Command::Server));
    }

    #[test]
    fn parses_client_mode_with_username() {
        let command = parse_command(args(&["bin", "mode=client", "username=alice"])).unwrap();

        assert!(matches!(
            command,
            Command::Client { username } if username == "alice"
        ));
    }

    #[test]
    fn rejects_missing_mode() {
        let error = parse_command(args(&["bin", "username=alice"])).unwrap_err();

        assert!(matches!(error, CliError::MissingMode));
    }

    #[test]
    fn rejects_missing_username_for_client_mode() {
        let error = parse_command(args(&["bin", "mode=client"])).unwrap_err();

        assert!(matches!(error, CliError::MissingUsername));
    }

    #[test]
    fn rejects_blank_username_for_client_mode() {
        let error = parse_command(args(&["bin", "mode=client", "username=   "])).unwrap_err();

        assert!(matches!(error, CliError::MissingUsername));
    }

    #[test]
    fn rejects_invalid_mode() {
        let error = parse_command(args(&["bin", "mode=invalid"])).unwrap_err();

        assert!(matches!(error, CliError::InvalidMode(mode) if mode == "invalid"));
    }
}
