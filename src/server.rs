use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

pub fn run_server() {
    let listener = TcpListener::bind("127.0.0.1:7878").unwrap();

    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));

    for stream in listener.incoming() {
        let stream = stream.unwrap();

        clients.lock().unwrap().push(stream.try_clone().unwrap());

        let clients_clone = Arc::clone(&clients);

        thread::spawn(move || {
            handle_client(stream, clients_clone);
        });
    }
}

fn handle_client(mut stream: TcpStream, clients: Arc<Mutex<Vec<TcpStream>>>) {
    let mut buffer = [0; 1024];
    let sender_addr = stream.peer_addr().unwrap();

    let bytes = stream.read(&mut buffer).unwrap();

    if bytes == 0 {
        return;
    }

    let username = String::from_utf8_lossy(&buffer[..bytes]).trim().to_string();
    let join_msg = format!("{} has entered the chat. Say hello!\n", username);
    println!("{}", join_msg.trim());

    broadcast(&clients, &join_msg, None);

    loop {
        let bytes = stream.read(&mut buffer).unwrap();

        if bytes == 0 {
            let leave_msg = format!("{} has left the chat\n", username);

            println!("{}", leave_msg.trim());
            broadcast(&clients, &leave_msg, None);
            break;
        }

        let msg = String::from_utf8_lossy(&buffer[..bytes]);
        print!("{}", msg);

        broadcast(&clients, &msg, Some(sender_addr));
    }
}

fn broadcast(
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
