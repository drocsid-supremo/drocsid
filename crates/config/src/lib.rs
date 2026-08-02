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

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            bind_addr: read_env("SERVER_BIND_ADDR", DEFAULT_SERVER_BIND_ADDR),
            connect_addr: read_env("SERVER_CONNECT_ADDR", DEFAULT_SERVER_CONNECT_ADDR),
            simulated_latency: Duration::from_millis(read_latency_ms_from_env()),
        }
    }
}

fn read_env(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn read_latency_ms_from_env() -> u64 {
    env::var("SERVER_SIMULATED_LATENCY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SERVER_SIMULATED_LATENCY_MS)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        DEFAULT_SERVER_BIND_ADDR, DEFAULT_SERVER_CONNECT_ADDR, DEFAULT_SERVER_SIMULATED_LATENCY_MS,
        ServerConfig,
    };

    #[test]
    fn default_server_config_values_are_stable() {
        let config = ServerConfig {
            bind_addr: DEFAULT_SERVER_BIND_ADDR.to_string(),
            connect_addr: DEFAULT_SERVER_CONNECT_ADDR.to_string(),
            simulated_latency: Duration::from_millis(DEFAULT_SERVER_SIMULATED_LATENCY_MS),
        };

        assert_eq!(config.bind_addr, "0.0.0.0:7878");
        assert_eq!(config.connect_addr, "127.0.0.1:7878");
        assert_eq!(config.simulated_latency, Duration::from_millis(0));
    }
}
