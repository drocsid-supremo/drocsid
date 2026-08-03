use std::time::Duration;

pub const DEFAULT_SERVER_BIND_ADDR: &str = "127.0.0.1:7878";
pub const DEFAULT_SERVER_CONNECT_ADDR: &str = "127.0.0.1:7878";
pub const DEFAULT_SERVER_SIMULATED_LATENCY_MS: u64 = 0;

#[derive(Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub connect_addr: String,
    pub simulated_latency: Duration,
}

impl ServerConfig {
    pub fn for_server(listen_addr: Option<String>, latency_ms: Option<u64>) -> Self {
        Self {
            bind_addr: listen_addr.unwrap_or_else(|| DEFAULT_SERVER_BIND_ADDR.to_string()),
            connect_addr: DEFAULT_SERVER_CONNECT_ADDR.to_string(),
            simulated_latency: Duration::from_millis(
                latency_ms.unwrap_or(DEFAULT_SERVER_SIMULATED_LATENCY_MS),
            ),
        }
    }

    pub fn for_client(connect_addr: Option<String>) -> Self {
        Self {
            bind_addr: DEFAULT_SERVER_BIND_ADDR.to_string(),
            connect_addr: connect_addr.unwrap_or_else(|| DEFAULT_SERVER_CONNECT_ADDR.to_string()),
            simulated_latency: Duration::from_millis(DEFAULT_SERVER_SIMULATED_LATENCY_MS),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::ServerConfig;

    #[test]
    fn default_server_config_values_are_stable() {
        let config = ServerConfig::for_server(None, None);

        assert_eq!(config.bind_addr, "127.0.0.1:7878");
        assert_eq!(config.connect_addr, "127.0.0.1:7878");
        assert_eq!(config.simulated_latency, Duration::from_millis(0));
    }

    #[test]
    fn applies_server_overrides() {
        let config = ServerConfig::for_server(Some("127.0.0.1:9000".to_string()), Some(25));

        assert_eq!(config.bind_addr, "127.0.0.1:9000");
        assert_eq!(config.simulated_latency, Duration::from_millis(25));
    }

    #[test]
    fn applies_client_connection_override() {
        let config = ServerConfig::for_client(Some("chat.example.com:7878".to_string()));

        assert_eq!(config.connect_addr, "chat.example.com:7878");
    }
}
