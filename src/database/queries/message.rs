use crate::database::{
    connection::connection,
};
use rusqlite::params;

pub fn create_message() -> Result<(), String> {
    let conn = connection().expect("failed to connect");

    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY,
                author TEXT NOT NULL,
                content TEXT NOT NULL
            );
        )",
        (),
    ).map_err(|e| e.to_string())?;

    Ok(())
}

pub fn insert_message(author: &str, content: &str) -> Result<(), String> {
    let conn = connection().expect("failed to connect");

    conn.execute(
        "INSERT INTO messages (author, content)
         VALUES (?1, ?2)",
        params![author, content],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}