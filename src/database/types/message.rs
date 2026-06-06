use crate::database::types::{
    user::User,
};

#[derive(Debug)]
pub struct Message {
    pub author: User,
    pub content: String,    
}