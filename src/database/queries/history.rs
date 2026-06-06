use rusqlite::Result;

use crate::database::connection::connection;

pub fn load_history() -> Result<Vec<String>, String> {
    let conn = connection().expect("failed to connect");

    let mut stmt = conn.prepare(
        "SELECT author, content FROM messages ORDER BY id ASC"
    )
    .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            let author: String = row.get(0)?;
            let content: String = row.get(1)?;

            Ok(format!("{}: {}", author, content))
        })
        .map_err(|e| e.to_string())?;

    let mut history = Vec::new();

    for msg in rows {
        history.push(msg.map_err(|e| e.to_string())?);
    }

    Ok(history)
}