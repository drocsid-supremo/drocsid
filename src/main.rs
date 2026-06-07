use std::env;

mod client;
mod peek_process;
mod server;

use crate::{client::run_client, peek_process::peek_process_ctx, server::run_server};

fn main() {
    let args: Vec<String> = env::args().collect();
    let (mode, username) = peek_process_ctx(&args);

    if mode == "server" {
        run_server();
    }

    if mode == "client" {
        run_client(username);
    }
}
