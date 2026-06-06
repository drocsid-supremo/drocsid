use rusqlite::{Connection, Result};

pub fn connection() -> Result<Connection> {
    Connection::open("chat.db")
}