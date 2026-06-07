use std::{
    io::{Read, Write, stdin, stdout},
    net::TcpStream,
    thread,
};

pub fn run_client(username: &str) {
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
