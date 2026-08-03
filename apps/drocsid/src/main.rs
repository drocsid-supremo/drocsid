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
    let command = match parse_command(std::env::args()) {
        Ok(command) => command,
        Err(error) => {
            let exit_code = error.exit_code();
            error.print()?;
            std::process::exit(exit_code);
        }
    };

    match command {
        Command::Server { listen, latency_ms } => {
            run_server(&ServerConfig::for_server(listen, latency_ms))?
        }
        Command::Client { username, connect } => {
            run_client(&username, &ServerConfig::for_client(connect))?
        }
    }

    Ok(())
}
