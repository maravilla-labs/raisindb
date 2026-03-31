//! Transaction statement AST definitions
//!
//! Defines the Abstract Syntax Tree for transaction control statements:
//! - BEGIN / BEGIN TRANSACTION
//! - COMMIT [WITH MESSAGE 'msg'] [ACTOR 'actor']

use serde::{Deserialize, Serialize};

/// Transaction control statement
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStatement {
    /// BEGIN or BEGIN TRANSACTION
    Begin,
    /// COMMIT [WITH MESSAGE '...'] [ACTOR '...']
    Commit {
        /// Optional commit message describing the changes
        message: Option<String>,
        /// Optional actor (user ID) performing the commit
        actor: Option<String>,
    },
    /// SET variable = value
    /// Session-level configuration within a transaction
    Set {
        /// Variable name (e.g., "validate_schema")
        variable: String,
        /// Variable value (e.g., "true", "false")
        value: String,
    },
}

impl TransactionStatement {
    /// Get a human-readable description of the transaction operation
    pub fn operation(&self) -> &'static str {
        match self {
            TransactionStatement::Begin => "BEGIN TRANSACTION",
            TransactionStatement::Commit { .. } => "COMMIT",
            TransactionStatement::Set { .. } => "SET",
        }
    }

    /// Create a BEGIN statement
    pub fn begin() -> Self {
        TransactionStatement::Begin
    }

    /// Create a COMMIT statement with optional message and actor
    pub fn commit(message: Option<String>, actor: Option<String>) -> Self {
        TransactionStatement::Commit { message, actor }
    }
}

impl std::fmt::Display for TransactionStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionStatement::Begin => write!(f, "BEGIN TRANSACTION"),
            TransactionStatement::Commit { message, actor } => {
                write!(f, "COMMIT")?;
                if let Some(msg) = message {
                    write!(f, " WITH MESSAGE '{}'", msg)?;
                }
                if let Some(act) = actor {
                    write!(f, " ACTOR '{}'", act)?;
                }
                Ok(())
            }
            TransactionStatement::Set { variable, value } => {
                write!(f, "SET {} = {}", variable, value)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_statement_display() {
        let begin = TransactionStatement::begin();
        assert_eq!(begin.to_string(), "BEGIN TRANSACTION");

        let commit_simple = TransactionStatement::commit(None, None);
        assert_eq!(commit_simple.to_string(), "COMMIT");

        let commit_with_message =
            TransactionStatement::commit(Some("Created new article".to_string()), None);
        assert_eq!(
            commit_with_message.to_string(),
            "COMMIT WITH MESSAGE 'Created new article'"
        );

        let commit_full = TransactionStatement::commit(
            Some("Updated user profile".to_string()),
            Some("user123".to_string()),
        );
        assert_eq!(
            commit_full.to_string(),
            "COMMIT WITH MESSAGE 'Updated user profile' ACTOR 'user123'"
        );
    }

    #[test]
    fn test_transaction_statement_operation() {
        assert_eq!(
            TransactionStatement::begin().operation(),
            "BEGIN TRANSACTION"
        );
        assert_eq!(
            TransactionStatement::commit(None, None).operation(),
            "COMMIT"
        );
    }
}
