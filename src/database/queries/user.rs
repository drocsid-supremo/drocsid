use crate::database::{
    connection::connection,
};

pub fn create_user() -> Result<(), String> {
    let conn = connection().expect("An error occurred to create user!");

    let query = conn.execute(
        "CREATE TABLE IF NOT EXISTS User (username TEXT NOT NULL, id INTEGER PRIMARY KEY)"
        , ()
    );

    match query {
        Ok(_) => { Ok(()) },
        Err(err) => Err(err.to_string())
    }
}

pub fn insert_user(username: &str) -> Result<(), String> {
    let conn = connection().expect("An error occurred to insert user!");

    let query = conn.execute("INSERT INTO User (username) VALUES (?1)", (username,));
    
    match query {
        Ok(_) => { Ok(()) },
        Err(err) => Err(err.to_string())
    }
}