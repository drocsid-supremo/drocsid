use std::{
    io::{Read, Write, stdin, stdout},
    net::TcpStream,
    thread,
};

use crate::error::AppError;

pub fn run_client(username: &str) -> Result<(), AppError> {
    if username.trim().is_empty() {
        return Err(AppError::MissingUsername);
    }

    let mut stream = TcpStream::connect("127.0.0.1:7878")?;
    let mut reader_stream = stream.try_clone()?;

    let join_msg = format!("{}\n", username);
    stream.write_all(join_msg.as_bytes())?;

    thread::spawn(move || {
        let mut buffer = [0; 1024];

        loop {
            let bytes = match reader_stream.read(&mut buffer) {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("failed to receive message from server: {error}");
                    break;
                }
            };

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
        let bytes_read = stdin().read_line(&mut input)?;

        if bytes_read == 0 {
            return Ok(());
        }

        let msg = format!("[{}]: {}\n", username, input.trim());

        print!("\x1B[1A\x1B[2K\r{}", msg);
        stdout().flush()?;

        if let Err(error) = stream.write_all(msg.as_bytes()) {
            if error.kind() == std::io::ErrorKind::BrokenPipe {
                return Err(AppError::ServerDisconnected);
            }

            return Err(error.into());
        }
    }
}
