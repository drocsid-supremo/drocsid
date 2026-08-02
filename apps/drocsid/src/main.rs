use std::process::ExitCode;

use drocsid_cli::{Command, parse_command};
use drocsid_config::ServerConfig;
use drocsid_server::run_server;
use drocsid_tui::run_client;

fn main() -> ExitCode {
    if let Err(error) = run() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let server_config = ServerConfig::from_env();
    let command = parse_command(std::env::args())?;

    match command {
        Command::Server => run_server(&server_config)?,
        Command::Client { username } => run_client(&username, &server_config)?,
    }

    Ok(())
}
