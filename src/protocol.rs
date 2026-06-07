pub const USERS_EVENT_PREFIX: &str = "__users__:";

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
