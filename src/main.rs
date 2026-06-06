use std::env;

mod client;
mod server;

fn peek_process_ctx(args: &Vec<String>) -> (&str, &str) {
    let mut mode = "";
    let mut username = "";

    for arg in args {
        if arg.starts_with("mode=") {
            mode = &arg[5..];
        }

        if arg.starts_with("username=") {
            username = &arg[9..];
        }
    }

    (mode, username)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (mode, username) = peek_process_ctx(&args);

    if mode == "server" {
        server::run_server();
    }

    if mode == "client" {
        client::run_client(username);
    }
}
