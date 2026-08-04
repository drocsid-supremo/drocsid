use std::io::{self, Read};

pub const CURRENT_PROTOCOL_VERSION: u8 = 1;
pub const FRAME_HEADER_BYTES: usize = 6;
pub const MAX_FRAME_PAYLOAD_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FrameType {
    Handshake = 1,
    Chat = 2,
    Presence = 3,
}

impl TryFrom<u8> for FrameType {
    type Error = FrameError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Handshake),
            2 => Ok(Self::Chat),
            3 => Ok(Self::Presence),
            _ => Err(FrameError::UnknownType(value)),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Frame {
    pub version: u8,
    pub kind: FrameType,
    pub payload: Vec<u8>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrameError {
    Io(io::ErrorKind),
    UnsupportedVersion(u8),
    UnknownType(u8),
    TooLarge(usize),
    InvalidUsername,
    Truncated,
}

pub fn encode_frame(kind: FrameType, payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > MAX_FRAME_PAYLOAD_BYTES {
        return Err(FrameError::TooLarge(payload.len()));
    }

    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&[
        CURRENT_PROTOCOL_VERSION,
        kind as u8,
        ((payload.len() >> 24) & 0xff) as u8,
        ((payload.len() >> 16) & 0xff) as u8,
        ((payload.len() >> 8) & 0xff) as u8,
        (payload.len() & 0xff) as u8,
    ]);
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Frame>, FrameError> {
    let mut header = [0; FRAME_HEADER_BYTES];
    if !read_header(reader, &mut header)? {
        return Ok(None);
    }

    let version = header[0];
    if version != CURRENT_PROTOCOL_VERSION {
        return Err(FrameError::UnsupportedVersion(version));
    }

    let kind = FrameType::try_from(header[1])?;
    let payload_len = u32::from_be_bytes([header[2], header[3], header[4], header[5]]) as usize;
    if payload_len > MAX_FRAME_PAYLOAD_BYTES {
        return Err(FrameError::TooLarge(payload_len));
    }

    let mut payload = vec![0; payload_len];
    reader
        .read_exact(&mut payload)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => FrameError::Truncated,
            kind => FrameError::Io(kind),
        })?;

    Ok(Some(Frame {
        version,
        kind,
        payload,
    }))
}

fn read_header<R: Read>(
    reader: &mut R,
    header: &mut [u8; FRAME_HEADER_BYTES],
) -> Result<bool, FrameError> {
    let mut read = 0;
    while read < header.len() {
        match reader.read(&mut header[read..]) {
            Ok(0) if read == 0 => return Ok(false),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(bytes) => read += bytes,
            Err(error) => return Err(FrameError::Io(error.kind())),
        }
    }
    Ok(true)
}

pub fn encode_presence(users: &[String]) -> Result<Vec<u8>, FrameError> {
    let mut payload = Vec::new();
    let count = u16::try_from(users.len()).map_err(|_| FrameError::TooLarge(usize::MAX))?;
    payload.extend_from_slice(&count.to_be_bytes());

    for user in users {
        if !is_valid_username(user) {
            return Err(FrameError::InvalidUsername);
        }
        let bytes = user.as_bytes();
        let length = u16::try_from(bytes.len()).map_err(|_| FrameError::TooLarge(bytes.len()))?;
        payload.extend_from_slice(&length.to_be_bytes());
        payload.extend_from_slice(bytes);
    }

    encode_frame(FrameType::Presence, &payload)
}

pub fn parse_presence(payload: &[u8]) -> Option<Vec<String>> {
    let count = u16::from_be_bytes(payload.get(..2)?.try_into().ok()?) as usize;
    let mut offset = 2;
    let mut users = Vec::with_capacity(count);

    for _ in 0..count {
        let length = u16::from_be_bytes(payload.get(offset..offset + 2)?.try_into().ok()?) as usize;
        offset += 2;
        let user = std::str::from_utf8(payload.get(offset..offset + length)?).ok()?;
        if !is_valid_username(user) {
            return None;
        }
        users.push(user.to_owned());
        offset += length;
    }

    (offset == payload.len()).then_some(users)
}

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
    use std::io::{self, Cursor, Read};

    use super::{
        CURRENT_PROTOCOL_VERSION, FrameError, FrameType, encode_frame, encode_presence,
        format_chat_message, is_valid_username, message_mentions_user, parse_chat_message,
        parse_presence, read_frame,
    };

    #[test]
    fn encodes_and_decodes_typed_frames() {
        let encoded = encode_frame(FrameType::Chat, b"hello\nworld").unwrap();
        let decoded = read_frame(&mut Cursor::new(encoded)).unwrap().unwrap();

        assert_eq!(decoded.version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(decoded.kind, FrameType::Chat);
        assert_eq!(decoded.payload, b"hello\nworld");
    }

    #[test]
    fn reads_frames_when_transport_returns_partial_chunks() {
        let encoded = encode_frame(FrameType::Chat, "x€".as_bytes()).unwrap();
        let mut reader = ChunkedReader::new(encoded, 2);

        let decoded = read_frame(&mut reader).unwrap().unwrap();

        assert_eq!(decoded.payload, "x€".as_bytes());
        assert_eq!(read_frame(&mut reader), Ok(None));
    }

    #[test]
    fn rejects_unsupported_versions_and_unknown_types() {
        let mut version = vec![9, FrameType::Chat as u8, 0, 0, 0, 0];
        assert_eq!(
            read_frame(&mut Cursor::new(&mut version)),
            Err(FrameError::UnsupportedVersion(9))
        );

        let mut kind = vec![CURRENT_PROTOCOL_VERSION, 99, 0, 0, 0, 0];
        assert_eq!(
            read_frame(&mut Cursor::new(&mut kind)),
            Err(FrameError::UnknownType(99))
        );
    }

    #[test]
    fn rejects_truncated_and_oversized_frames() {
        let mut truncated = vec![
            CURRENT_PROTOCOL_VERSION,
            FrameType::Chat as u8,
            0,
            0,
            0,
            3,
            b'x',
        ];
        assert_eq!(
            read_frame(&mut Cursor::new(&mut truncated)),
            Err(FrameError::Truncated)
        );

        let too_large = vec![0; super::MAX_FRAME_PAYLOAD_BYTES + 1];
        assert_eq!(
            encode_frame(FrameType::Chat, &too_large),
            Err(FrameError::TooLarge(too_large.len()))
        );

        let mut oversized_header = vec![
            CURRENT_PROTOCOL_VERSION,
            FrameType::Chat as u8,
            0,
            0,
            0x40,
            1,
        ];
        assert_eq!(
            read_frame(&mut Cursor::new(&mut oversized_header)),
            Err(FrameError::TooLarge(super::MAX_FRAME_PAYLOAD_BYTES + 1))
        );
    }

    #[test]
    fn distinguishes_clean_eof_from_truncated_header() {
        assert_eq!(read_frame(&mut Cursor::new(Vec::<u8>::new())), Ok(None));

        let mut truncated_header = vec![CURRENT_PROTOCOL_VERSION, FrameType::Chat as u8];
        assert_eq!(
            read_frame(&mut Cursor::new(&mut truncated_header)),
            Err(FrameError::Truncated)
        );
    }

    #[test]
    fn encodes_and_parses_presence_without_a_reserved_prefix() {
        let users = vec!["alice,ops".to_string(), "bob".to_string()];
        let frame = encode_presence(&users).unwrap();
        let decoded = read_frame(&mut Cursor::new(frame)).unwrap().unwrap();

        assert_eq!(decoded.kind, FrameType::Presence);
        assert_eq!(parse_presence(&decoded.payload), Some(users));
    }

    #[test]
    fn supports_presence_payloads_larger_than_chat_messages() {
        let users = vec!["x".repeat(4096)];
        let frame = encode_presence(&users).unwrap();
        let decoded = read_frame(&mut Cursor::new(frame)).unwrap().unwrap();

        assert_eq!(parse_presence(&decoded.payload), Some(users));
    }

    #[test]
    fn encodes_and_parses_empty_presence() {
        let frame = encode_presence(&[]).unwrap();
        let decoded = read_frame(&mut Cursor::new(frame)).unwrap().unwrap();

        assert_eq!(parse_presence(&decoded.payload), Some(Vec::new()));
    }

    #[test]
    fn rejects_malformed_presence_payloads() {
        assert_eq!(parse_presence(&[]), None);
        assert_eq!(parse_presence(&[0, 1, 0]), None);
        assert_eq!(
            parse_presence(&[0, 0, 0, 1, b'a', b'e', b'x', b't', b'r', b'a']),
            None
        );
    }

    #[test]
    fn rejects_invalid_usernames_in_presence_payloads() {
        let invalid_users = ["", "alice\nadmin", "alice\u{1b}[2J"];

        for user in invalid_users {
            assert_eq!(
                encode_presence(&[user.to_string()]),
                Err(FrameError::InvalidUsername),
                "user: {user:?}"
            );
        }
    }

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

    struct ChunkedReader {
        bytes: Vec<u8>,
        offset: usize,
        chunk_size: usize,
    }

    impl ChunkedReader {
        fn new(bytes: Vec<u8>, chunk_size: usize) -> Self {
            Self {
                bytes,
                offset: 0,
                chunk_size,
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.offset == self.bytes.len() {
                return Ok(0);
            }

            let length = self
                .chunk_size
                .min(buffer.len())
                .min(self.bytes.len() - self.offset);
            buffer[..length].copy_from_slice(&self.bytes[self.offset..self.offset + length]);
            self.offset += length;
            Ok(length)
        }
    }
}
