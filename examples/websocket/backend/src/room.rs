use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_USERS: usize = 256;

#[derive(Clone, Serialize)]
pub(crate) struct User {
    id: String,
    username: String,
}

#[derive(Serialize)]
pub(crate) struct Message {
    id: String,
    username: String,
    content: String,
    timestamp: String,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
pub(crate) struct MembershipEvent {
    user: User,
    message: Message,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JoinRequest {
    pub(crate) username: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageRequest {
    username: String,
    content: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct ChatError {
    code: &'static str,
    message: &'static str,
}

impl ChatError {
    pub(crate) fn invalid_payload() -> Self {
        Self {
            code: "invalid_payload",
            message: "Event payload has an invalid shape",
        }
    }
}

#[derive(Default)]
pub(crate) struct ChatRoom {
    users: BTreeMap<String, User>,
    sequence: u64,
}

impl ChatRoom {
    pub(crate) fn join(
        &mut self,
        id: &str,
        username: String,
        timestamp: String,
    ) -> Result<Option<MembershipEvent>, ChatError> {
        if !valid_text(&username, 64, 256)
            || username.trim() != username
            || username.chars().any(char::is_control)
        {
            return Err(ChatError {
                code: "invalid_username",
                message: "Username must contain 1 to 64 characters without surrounding whitespace or control characters",
            });
        }
        if let Some(user) = self.users.get(id) {
            return if user.username == username {
                Ok(None)
            } else {
                Err(ChatError {
                    code: "username_mismatch",
                    message: "Disconnect before changing a joined username",
                })
            };
        }
        if self.users.len() >= MAX_USERS {
            return Err(ChatError {
                code: "room_full",
                message: "The reference chat room is full",
            });
        }
        let user = User {
            id: id.to_owned(),
            username,
        };
        let message = self.make_message(
            "System".to_owned(),
            format!("{} joined the chat room", user.username),
            "system",
            timestamp,
        );
        self.users.insert(id.to_owned(), user.clone());
        Ok(Some(MembershipEvent { user, message }))
    }

    pub(crate) fn users(&self) -> Vec<User> {
        self.users.values().cloned().collect()
    }

    pub(crate) fn message(
        &mut self,
        id: &str,
        request: MessageRequest,
        timestamp: String,
    ) -> Result<Message, ChatError> {
        let user = self.users.get(id).ok_or(ChatError {
            code: "not_joined",
            message: "Join the chat room before sending a message",
        })?;
        if user.username != request.username {
            return Err(ChatError {
                code: "username_mismatch",
                message: "Message username must match this connection",
            });
        }
        if !valid_text(&request.content, 4096, 16384) {
            return Err(ChatError {
                code: "invalid_message",
                message: "Message must contain 1 to 4096 characters and at most 16384 bytes",
            });
        }
        Ok(self.make_message(request.username, request.content, "user", timestamp))
    }

    pub(crate) fn leave(&mut self, id: &str, timestamp: String) -> Option<MembershipEvent> {
        let user = self.users.remove(id)?;
        let message = self.make_message(
            "System".to_owned(),
            format!("{} left the chat room", user.username),
            "system",
            timestamp,
        );
        Some(MembershipEvent { user, message })
    }

    fn make_message(
        &mut self,
        username: String,
        content: String,
        kind: &'static str,
        timestamp: String,
    ) -> Message {
        self.sequence = self.sequence.wrapping_add(1);
        Message {
            id: format!("chat-{}", self.sequence),
            username,
            content,
            timestamp,
            kind,
        }
    }
}

fn valid_text(text: &str, characters: usize, bytes: usize) -> bool {
    !text.trim().is_empty() && text.len() <= bytes && text.chars().count() <= characters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_bound_to_the_joined_connection_and_disconnect_removes_it() {
        let mut room = ChatRoom::default();
        let request = |username: &str| MessageRequest {
            username: username.into(),
            content: "Hello".into(),
        };
        assert_eq!(
            room.message("a", request("Alice"), String::new())
                .err()
                .unwrap()
                .code,
            "not_joined"
        );
        assert!(
            room.join("a", "Alice".into(), String::new())
                .unwrap()
                .is_some()
        );
        assert!(
            room.join("a", "Alice".into(), String::new())
                .unwrap()
                .is_none()
        );
        assert!(room.join("a", "Bob".into(), String::new()).is_err());
        assert_eq!(
            room.message("a", request("Bob"), String::new())
                .err()
                .unwrap()
                .code,
            "username_mismatch"
        );
        assert!(room.message("a", request("Alice"), String::new()).is_ok());
        assert!(room.leave("a", String::new()).is_some());
        assert!(room.leave("a", String::new()).is_none());
        assert!(room.users().is_empty());
    }

    #[test]
    fn text_and_room_limits_are_enforced() {
        let mut room = ChatRoom::default();
        for username in ["", " ", " Alice", "Alice ", "A\nB", &"字".repeat(65)] {
            assert!(
                room.join("invalid", username.into(), String::new())
                    .is_err()
            );
        }
        room.join("a", "Alice".into(), String::new()).unwrap();
        for content in ["".to_owned(), " ".to_owned(), "字".repeat(4097)] {
            assert!(
                room.message(
                    "a",
                    MessageRequest {
                        username: "Alice".into(),
                        content
                    },
                    String::new()
                )
                .is_err()
            );
        }
        for id in 1..MAX_USERS {
            room.join(&id.to_string(), "User".into(), String::new())
                .unwrap();
        }
        assert_eq!(
            room.join("overflow", "User".into(), String::new())
                .err()
                .unwrap()
                .code,
            "room_full"
        );
    }
}
