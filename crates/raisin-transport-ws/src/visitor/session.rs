// SPDX-License-Identifier: BSL-1.1

//! Visitor session secrets, keys and conversation ids.
//!
//! The SECRET is what the browser keeps (sessionStorage) and presents to
//! resume its session after a reconnect: 256 random bits, lowercase hex. The
//! KEY names the session's home (`/visitors/<key>`): the first 128 bits of
//! SHA-256(secret). The key is visible in paths and participant ids; knowing
//! it proves nothing, because only the secret binds a connection to the home.

use sha2::{Digest, Sha256};

/// Length of a session secret in hex characters (256 bits).
pub const SECRET_HEX_LEN: usize = 64;

/// A fresh session secret.
pub fn new_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// The session key a secret names, or `None` for anything that is not a
/// secret this server could have minted.
pub fn key_of(secret: &str) -> Option<String> {
    let well_formed = secret.len() == SECRET_HEX_LEN
        && secret
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !well_formed {
        return None;
    }
    let digest = Sha256::digest(secret.as_bytes());
    Some(hex::encode(&digest[..16]))
}

/// A fresh conversation id. Server-minted and unguessable: the conversation
/// id is also the event channel (`chat:<id>`) and the agent-side thread name.
pub fn new_conversation_id() -> String {
    format!("vchat-{}", uuid::Uuid::new_v4().simple())
}

/// Whether `id` has the shape [`new_conversation_id`] produces.
pub fn is_conversation_id(id: &str) -> bool {
    id.strip_prefix("vchat-").is_some_and(|rest| {
        rest.len() == 32
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::auth::visitor::is_valid_visitor_key;

    #[test]
    fn a_secret_names_a_stable_valid_key() {
        let secret = new_secret();
        assert_eq!(secret.len(), SECRET_HEX_LEN);
        let key = key_of(&secret).unwrap();
        assert_eq!(key.len(), 32);
        assert!(is_valid_visitor_key(&key));
        assert_eq!(key_of(&secret).unwrap(), key, "deterministic");
        assert_ne!(key_of(&new_secret()).unwrap(), key, "and unique");
        assert!(!secret.contains(&key), "the key does not reveal the secret");
    }

    #[test]
    fn anything_else_is_not_a_secret() {
        assert!(key_of("").is_none());
        assert!(key_of("../../users/alice").is_none());
        assert!(key_of(&"A".repeat(64)).is_none());
        assert!(key_of(&"a".repeat(63)).is_none());
    }

    #[test]
    fn conversation_ids_round_trip() {
        let id = new_conversation_id();
        assert!(is_conversation_id(&id));
        assert!(!is_conversation_id("chat-123"));
        assert!(!is_conversation_id("vchat-../x"));
    }
}
