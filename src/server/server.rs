use crate::server::{
    handler::connection::handle_connection,
};

use std::{
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

pub fn run_server() {
    let listener = TcpListener::bind("0.0.0.0:7878").unwrap();

    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));

    for stream in listener.incoming() {
        let stream = stream.unwrap();

        clients.lock().unwrap().push(stream.try_clone().unwrap());

        let clients_clone = Arc::clone(&clients);

        thread::spawn(move || {
            handle_connection(stream, clients_clone);
        });
    }
}