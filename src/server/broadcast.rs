use std::{
    io::Write,
    net::TcpStream,
    sync::{Arc, Mutex},
};

pub fn broadcast(
    clients: &Arc<Mutex<Vec<TcpStream>>>,
    msg: &str,
    exclude_addr: Option<std::net::SocketAddr>,
) {
    let mut clients = clients.lock().unwrap();

    for client in clients.iter_mut() {
        if exclude_addr.is_some_and(|addr| client.peer_addr().ok() == Some(addr)) {
            continue;
        }

        client.write_all(msg.as_bytes()).unwrap();
    }
}