use std::env;

mod client;
mod server;
mod peek_process;

use crate::{peek_process::peek_process_ctx, server::server::{run_server}, client::client::{run_client}};

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
