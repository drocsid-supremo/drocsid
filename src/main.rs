use std::{env, process::ExitCode};

mod client;
mod error;
mod peek_process;
mod server;

use crate::{
    client::run_client,
    error::AppError,
    peek_process::{Command, parse_command},
    server::run_server,
};

fn main() -> ExitCode {
    if let Err(error) = run() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn run() -> Result<(), AppError> {
    let args: Vec<String> = env::args().collect();
    let command = parse_command(&args)?;

    match command {
        Command::Server => run_server(),
        Command::Client { username } => run_client(&username),
    }
}
