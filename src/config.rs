use std::{env, time::Duration};

pub const DEFAULT_SERVER_BIND_ADDR: &str = "0.0.0.0:7878";
pub const DEFAULT_SERVER_CONNECT_ADDR: &str = "127.0.0.1:7878";
pub const DEFAULT_SERVER_SIMULATED_LATENCY_MS: u64 = 0;

#[derive(Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub connect_addr: String,
    pub simulated_latency: Duration,
}

pub struct AppConfig {
    pub server: ServerConfig,
}

impl AppConfig {
    pub fn load() -> Self {
        let _ = dotenvy::dotenv();

        Self {
            server: ServerConfig {
                bind_addr: read_env("SERVER_BIND_ADDR", DEFAULT_SERVER_BIND_ADDR),
                connect_addr: read_env("SERVER_CONNECT_ADDR", DEFAULT_SERVER_CONNECT_ADDR),
                simulated_latency: Duration::from_millis(read_latency_ms()),
            },
        }
    }
}

fn read_env(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn read_latency_ms() -> u64 {
    env::var("SERVER_SIMULATED_LATENCY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SERVER_SIMULATED_LATENCY_MS)
}
