use std::process::ExitCode;

use drocsid_cli::{Command, parse_command};
use drocsid_config::ServerConfig;
use drocsid_server::run_server;
use drocsid_tui::run_client;
use tracing_subscriber::{EnvFilter, fmt};

fn main() -> ExitCode {
    if let Err(error) = init_tracing() {
        eprintln!("failed to initialize logging: {error}");
        return ExitCode::FAILURE;
    }

    if let Err(error) = run() {
        tracing::error!(error = %error, "application failed");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn init_tracing() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json = std::env::var("DROCSID_LOG_FORMAT")
        .map(|value| value.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    if json {
        fmt().with_env_filter(filter).json().try_init()?;
    } else {
        fmt().with_env_filter(filter).try_init()?;
    }

    Ok(())
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
