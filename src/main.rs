use std::env;

mod client;
mod server;
mod peek_process;
mod database;

use crate::{
    client::client::run_client, 
    database::{init::init_db}, 
    peek_process::peek_process_ctx, 
    server::server::run_server
};

fn main() {
    init_db();
    
    let args: Vec<String> = env::args().collect();
    let (mode, username) = peek_process_ctx(&args);

    if mode == "server" {
        run_server();
    } 

    if mode == "client" {
        run_client(username);
    }
}
