// SPDX-License-Identifier: BSL-1.1

//! WebSocket connection state management
//!
//! This module handles per-connection state including request tracking,
//! subscriptions, and concurrency control.

mod errors;
pub(crate) mod path_matching;
mod state;
mod subscriptions;

// Re-export public API
pub use errors::{AcquireError, SendError, TransactionError};
pub use state::ConnectionState;

#[cfg(test)]
mod tests {
    use super::*;
    use path_matching::path_matches;

    #[test]
    fn test_path_matching() {
        // Exact match
        assert!(path_matches("/folder/file.txt", "/folder/file.txt"));

        // Shell glob style wildcards
        assert!(path_matches("/folder/file.txt", "/folder/*"));
        assert!(path_matches("/folder/subfolder/file.txt", "/folder/**"));
        assert!(!path_matches("/other/file.txt", "/folder/*"));

        // SQL LIKE style wildcards (% converted to *)
        assert!(path_matches("/posts/123", "/posts/%"));
        assert!(path_matches("/users/carol", "/users/%"));
        assert!(!path_matches("/posts/123", "/users/%"));

        // Mixed patterns
        assert!(path_matches("/posts/123/comments", "/posts/%/comments"));
    }

    #[test]
    fn test_connection_state_basic() {
        let state = ConnectionState::new("tenant1".to_string(), Some("repo1".to_string()), 4, 100);

        assert_eq!(state.tenant_id, "tenant1");
        assert_eq!(state.repository, Some("repo1".to_string()));
        assert!(!state.is_authenticated());
        assert_eq!(state.get_credits(), 100);
    }

    #[test]
    fn test_credits() {
        let state = ConnectionState::new("tenant1".to_string(), None, 4, 100);

        assert!(state.try_consume_credits(50));
        assert_eq!(state.get_credits(), 50);

        assert!(!state.try_consume_credits(100));
        assert_eq!(state.get_credits(), 50);

        state.add_credits(30);
        assert_eq!(state.get_credits(), 80);
    }
}
