use crate::error::AppError;

pub enum Command {
    Server,
    Client { username: String },
    Tui,
}

pub fn parse_command(args: impl IntoIterator<Item = String>) -> Result<Command, AppError> {
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

    match mode.as_deref().ok_or(AppError::MissingMode)? {
        "server" => Ok(Command::Server),
        "client" => {
            let username = username
                .filter(|value| !value.trim().is_empty())
                .ok_or(AppError::MissingUsername)?;

            Ok(Command::Client { username })
        }
        "tui" => Ok(Command::Tui),
        other => Err(AppError::InvalidMode(other.to_string())),
    }
}
