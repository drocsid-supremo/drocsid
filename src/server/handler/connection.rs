use std::{net::{TcpStream}, sync::{Arc, Mutex}};
use std::io::{Read};

use crate::{database::queries::{message::insert_message}, server::broadcast::broadcast};

pub fn handle_connection(mut stream: TcpStream, clients: Arc<Mutex<Vec<TcpStream>>>) {
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

        let msg = String::from_utf8_lossy(&buffer[..bytes]).to_string();
        insert_message(&username, &msg).unwrap();

        print!("{}", msg);

        broadcast(&clients, &msg, Some(sender_addr));
    }
}
