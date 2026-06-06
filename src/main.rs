use std::{
    env,
    io::{Read, Write, stdin, stdout},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

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
        run_server();
    }

    if mode == "client" {
        run_client(username);
    }
}

fn run_server() {
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

fn run_client(username: &str) {
    if username.is_empty() {
        println!("missing username");
        return;
    }

    let mut stream = TcpStream::connect("127.0.0.1:7878").unwrap();

    let mut reader_stream = stream.try_clone().unwrap();

    let join_msg = format!("{}\n", username);
    stream.write_all(join_msg.as_bytes()).unwrap();

    thread::spawn(move || {
        let mut buffer = [0; 1024];

        loop {
            let bytes = reader_stream.read(&mut buffer).unwrap();

            if bytes == 0 {
                println!("server disconnected");
                break;
            }

            let msg = String::from_utf8_lossy(&buffer[..bytes]);

            print!("{}", msg);
        }
    });

    loop {
        let mut input = String::new();

        stdin().read_line(&mut input).unwrap();

        let msg = format!("[{}]: {}\n", username, input.trim());

        print!("\x1B[1A\x1B[2K\r{}", msg);
        stdout().flush().unwrap();

        stream.write_all(msg.as_bytes()).unwrap();
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
