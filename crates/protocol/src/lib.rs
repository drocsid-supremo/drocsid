pub const USERS_EVENT_PREFIX: &str = "__users__:";

pub fn is_valid_username(username: &str) -> bool {
    !username.trim().is_empty() && !username.chars().any(char::is_control)
}

pub fn format_chat_message(username: &str, timestamp: &str, content: &str) -> String {
    format!("[{username}]({timestamp}): {content}")
}

pub fn parse_chat_message(message: &str) -> Option<(&str, &str, &str)> {
    let close_user = message.find("](")?;
    let close_time = message[close_user + 2..].find("): ")? + close_user + 2;
    let username = message.get(1..close_user)?;
    let timestamp = message.get(close_user + 2..close_time)?;
    let body = message.get(close_time + 3..)?;

    message
        .starts_with('[')
        .then_some((username, timestamp, body))
}

pub fn build_users_event(users: &[String]) -> String {
    format!("{USERS_EVENT_PREFIX}{}\n", users.join(","))
}

pub fn parse_users_event(line: &str) -> Option<Vec<String>> {
    let payload = line.strip_prefix(USERS_EVENT_PREFIX)?;
    if payload.is_empty() {
        return Some(Vec::new());
    }

    Some(
        payload
            .split(',')
            .map(str::trim)
            .filter(|user| !user.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

pub fn message_mentions_user(text: &str, username: &str) -> bool {
    let needle = format!("@{username}");
    let mut search_start = 0;

    while let Some(relative_index) = text[search_start..].find(&needle) {
        let index = search_start + relative_index;
        let after = text[index + needle.len()..].chars().next();

        if after.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_') {
            return true;
        }

        search_start = index + needle.len();
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{
        USERS_EVENT_PREFIX, build_users_event, format_chat_message, is_valid_username,
        message_mentions_user, parse_chat_message, parse_users_event,
    };

    #[test]
    fn formats_and_parses_chat_message() {
        let message = format_chat_message("alice", "12:34", "hello");
        let parsed = parse_chat_message(&message).unwrap();

        assert_eq!(message, "[alice](12:34): hello");
        assert_eq!(parsed, ("alice", "12:34", "hello"));
    }

    #[test]
    fn rejects_invalid_chat_message_shape() {
        assert_eq!(parse_chat_message("alice(12:34): hello"), None);
        assert_eq!(parse_chat_message("[alice](12:34 hello"), None);
        assert_eq!(parse_chat_message("[alice] 12:34: hello"), None);
    }

    #[test]
    fn builds_and_parses_users_event() {
        let users = vec!["alice".to_string(), "bob".to_string()];
        let event = build_users_event(&users);

        assert_eq!(event, format!("{USERS_EVENT_PREFIX}alice,bob\n"));
        assert_eq!(parse_users_event(&event), Some(users));
    }

    #[test]
    fn parses_empty_users_event() {
        assert_eq!(parse_users_event(USERS_EVENT_PREFIX), Some(Vec::new()));
    }

    #[test]
    fn ignores_blank_users_when_parsing_event() {
        let parsed = parse_users_event("__users__: alice, , bob ,,").unwrap();

        assert_eq!(parsed, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn detects_mentions_with_word_boundary() {
        assert!(message_mentions_user("hello @alice", "alice"));
        assert!(message_mentions_user("@alice, are you there?", "alice"));
    }

    #[test]
    fn rejects_partial_mentions() {
        assert!(!message_mentions_user("hello @alice1", "alice"));
        assert!(!message_mentions_user("hello @alice_name", "alice"));
        assert!(!message_mentions_user("hello alice", "alice"));
    }

    #[test]
    fn rejects_usernames_with_control_characters() {
        assert!(!is_valid_username("alice\u{1b}[2J"));
        assert!(!is_valid_username("alice\nadmin"));
        assert!(!is_valid_username("alice\radmin"));
        assert!(!is_valid_username("alice\tadmin"));
    }

    #[test]
    fn accepts_usernames_with_printable_characters() {
        assert!(is_valid_username("alice_42"));
        assert!(is_valid_username("café"));
    }
}
