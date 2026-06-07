use crate::protocol::message_mentions_user;

const MESSAGE_LIMIT: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Pending,
    Confirmed,
}

pub struct ChatMessage {
    pub text: String,
    pub state: MessageState,
}

pub struct ChatApp {
    pub username: String,
    pub input: String,
    pub messages: Vec<ChatMessage>,
    pub connected_users: Vec<String>,
    pub mention_selection: usize,
    pub connected: bool,
    pub exit_notice: Option<String>,
    pub should_quit: bool,
}

impl ChatApp {
    pub fn new(username: &str, server_addr: &str) -> Self {
        Self {
            username: username.to_string(),
            input: String::new(),
            messages: vec![
                ChatMessage {
                    text: format!("[system] Connected to {server_addr}"),
                    state: MessageState::Confirmed,
                },
                ChatMessage {
                    text: "[system] Type a message and press Enter.".to_string(),
                    state: MessageState::Confirmed,
                },
            ],
            connected_users: vec![username.to_string()],
            mention_selection: 0,
            connected: true,
            exit_notice: None,
            should_quit: false,
        }
    }

    pub fn push_message(&mut self, message: impl Into<String>, state: MessageState) {
        self.messages.push(ChatMessage {
            text: message.into(),
            state,
        });

        if self.messages.len() > MESSAGE_LIMIT {
            let overflow = self.messages.len() - MESSAGE_LIMIT;
            self.messages.drain(0..overflow);
        }
    }

    pub fn receive_message(&mut self, message: String) {
        if !self.confirm_message(&message) {
            self.push_message(message, MessageState::Confirmed);
        }
    }

    pub fn update_connected_users(&mut self, users: Vec<String>) {
        self.connected_users = users;

        let candidates_len = self.mention_candidates().len();
        if candidates_len == 0 {
            self.mention_selection = 0;
        } else if self.mention_selection >= candidates_len {
            self.mention_selection = candidates_len - 1;
        }
    }

    pub fn begin_shutdown(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.connected = false;
        self.push_message(format!("[system] {reason}"), MessageState::Confirmed);
        self.exit_notice = Some(reason);
    }

    pub fn mark_offline(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.connected = false;
        self.push_message(format!("[system] {reason}"), MessageState::Confirmed);
    }

    pub fn mention_candidates(&self) -> Vec<String> {
        let Some(query) = active_mention_query(&self.input) else {
            return Vec::new();
        };

        let query_lower = query.to_ascii_lowercase();
        self.connected_users
            .iter()
            .filter(|user| user.to_ascii_lowercase().starts_with(&query_lower))
            .cloned()
            .collect()
    }

    pub fn append_input(&mut self, character: char) {
        self.input.push(character);
        self.reset_mention_selection();
    }

    pub fn pop_input(&mut self) {
        self.input.pop();
        self.reset_mention_selection();
    }

    pub fn take_input(&mut self) -> String {
        let content = self.input.trim().to_string();
        self.input.clear();
        content
    }

    pub fn has_exit_notice(&self) -> bool {
        self.exit_notice.is_some()
    }

    pub fn select_previous_mention(&mut self) {
        let candidates = self.mention_candidates();
        if candidates.is_empty() {
            return;
        }

        if self.mention_selection == 0 {
            self.mention_selection = candidates.len() - 1;
        } else {
            self.mention_selection -= 1;
        }
    }

    pub fn select_next_mention(&mut self) {
        let candidates = self.mention_candidates();
        if candidates.is_empty() {
            return;
        }

        self.mention_selection = (self.mention_selection + 1) % candidates.len();
    }

    pub fn apply_selected_mention(&mut self) {
        let candidates = self.mention_candidates();
        if candidates.is_empty() {
            return;
        }

        let mention = &candidates[self.mention_selection];
        let Some(at_index) = self.input.rfind('@') else {
            return;
        };

        self.input.truncate(at_index);
        self.input.push('@');
        self.input.push_str(mention);
        self.input.push(' ');
        self.mention_selection = 0;
    }

    pub fn message_mentions_current_user(&self, text: &str) -> bool {
        message_mentions_user(text, &self.username)
    }

    fn confirm_message(&mut self, message: &str) -> bool {
        if let Some(entry) = self
            .messages
            .iter_mut()
            .find(|entry| entry.text == message && entry.state == MessageState::Pending)
        {
            entry.state = MessageState::Confirmed;
            return true;
        }

        false
    }

    fn reset_mention_selection(&mut self) {
        self.mention_selection = 0;
    }
}

fn active_mention_query(input: &str) -> Option<&str> {
    let at_index = input.rfind('@')?;
    let mention = input.get(at_index + 1..)?;

    if mention.contains(char::is_whitespace) {
        return None;
    }

    if at_index > 0 {
        let previous = input[..at_index].chars().last()?;
        if !previous.is_whitespace() {
            return None;
        }
    }

    Some(mention)
}
