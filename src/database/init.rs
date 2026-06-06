use crate::database::queries::{message::create_message, user::create_user};

pub fn init_db() -> () {
    create_user().expect("failed to create user table");
    create_message().expect("failed to create message table");
}