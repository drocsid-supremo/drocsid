use std::process::ExitCode;

mod cli;
mod client;
mod config;
mod error;
mod protocol;
mod server;
mod tui;

use crate::{
    cli::{Command, parse_command},
    client::run_client,
    config::AppConfig,
    error::AppError,
    server::run_server,
    tui::run_tui,
};

fn main() -> ExitCode {
    if let Err(error) = run() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn run() -> Result<(), AppError> {
    let config = AppConfig::load();
    let command = parse_command(std::env::args())?;

    match command {
        Command::Server => run_server(&config.server),
        Command::Client { username } => run_client(&username, &config.server),
        Command::Tui => run_tui(),
    }
}
