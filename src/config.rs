use std::env;

pub const DEFAULT_SERVER_BIND_ADDR: &str = "0.0.0.0:7878";
pub const DEFAULT_SERVER_CONNECT_ADDR: &str = "127.0.0.1:7878";

pub fn load_env() {
    let _ = dotenvy::dotenv();
}

pub fn server_bind_addr() -> String {
    env::var("SERVER_BIND_ADDR").unwrap_or_else(|_| DEFAULT_SERVER_BIND_ADDR.to_string())
}

pub fn server_connect_addr() -> String {
    env::var("SERVER_CONNECT_ADDR").unwrap_or_else(|_| DEFAULT_SERVER_CONNECT_ADDR.to_string())
}
