//! RESTORE statement AST definitions
//!
//! Defines the Abstract Syntax Tree for node restore statements:
//! - RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2
//! - RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5
//! - RESTORE NODE id='uuid' TO REVISION HEAD~2 TRANSLATIONS ('en')

use super::branch::RevisionRef;
use super::order::NodeReference;
use serde::{Deserialize, Serialize};

/// RESTORE statement for restoring nodes to a previous revision state
///
/// ```sql
/// RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2
/// RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5
/// RESTORE NODE id='uuid' TO REVISION HEAD~2 TRANSLATIONS ('en', 'de')
/// ```
///
/// This restores a node (and optionally its descendants) to its state at a previous revision.
/// The node stays at its current path - this is an in-place restore, not a copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreStatement {
    /// The node to restore (by path or id)
    pub node: NodeReference,
    /// The revision to restore from
    pub revision: RevisionRef,
    /// Whether to restore children (RESTORE TREE NODE)
    pub recursive: bool,
    /// Specific translations to restore (None = all translations)
    pub translations: Option<Vec<String>>,
}

impl RestoreStatement {
    /// Create a new RESTORE statement (single node)
    pub fn new(node: NodeReference, revision: RevisionRef) -> Self {
        Self {
            node,
            revision,
            recursive: false,
            translations: None,
        }
    }

    /// Create a new RESTORE TREE statement (recursive)
    pub fn new_tree(node: NodeReference, revision: RevisionRef) -> Self {
        Self {
            node,
            revision,
            recursive: true,
            translations: None,
        }
    }

    /// Create a RESTORE statement with all options
    pub fn with_options(
        node: NodeReference,
        revision: RevisionRef,
        recursive: bool,
        translations: Option<Vec<String>>,
    ) -> Self {
        Self {
            node,
            revision,
            recursive,
            translations,
        }
    }

    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        if self.recursive {
            "RESTORE TREE NODE"
        } else {
            "RESTORE NODE"
        }
    }
}

impl std::fmt::Display for RestoreStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.recursive {
            write!(f, "RESTORE TREE NODE {}", self.node)?;
        } else {
            write!(f, "RESTORE NODE {}", self.node)?;
        }

        write!(f, " TO REVISION {}", self.revision)?;

        // Optional TRANSLATIONS clause
        if let Some(translations) = &self.translations {
            write!(f, " TRANSLATIONS (")?;
            for (i, locale) in translations.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "'{}'", locale)?;
            }
            write!(f, ")")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restore_statement_display() {
        let stmt = RestoreStatement::new(
            NodeReference::path("/articles/my-article"),
            RevisionRef::head_relative(2),
        );
        assert_eq!(
            stmt.to_string(),
            "RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2"
        );
    }

    #[test]
    fn test_restore_tree_statement_display() {
        let stmt = RestoreStatement::new_tree(
            NodeReference::path("/products/category"),
            RevisionRef::head_relative(5),
        );
        assert_eq!(
            stmt.to_string(),
            "RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5"
        );
    }

    #[test]
    fn test_restore_statement_with_id() {
        let stmt =
            RestoreStatement::new(NodeReference::id("uuid-123"), RevisionRef::head_relative(3));
        assert_eq!(
            stmt.to_string(),
            "RESTORE NODE id='uuid-123' TO REVISION HEAD~3"
        );
    }

    #[test]
    fn test_restore_statement_with_translations() {
        let stmt = RestoreStatement::with_options(
            NodeReference::path("/articles/my-article"),
            RevisionRef::head_relative(2),
            false,
            Some(vec!["en".to_string(), "de".to_string()]),
        );
        assert_eq!(
            stmt.to_string(),
            "RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2 TRANSLATIONS ('en', 'de')"
        );
    }

    #[test]
    fn test_restore_tree_with_translations() {
        let stmt = RestoreStatement::with_options(
            NodeReference::path("/products/category"),
            RevisionRef::head_relative(5),
            true,
            Some(vec!["fr".to_string()]),
        );
        assert_eq!(
            stmt.to_string(),
            "RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5 TRANSLATIONS ('fr')"
        );
    }

    #[test]
    fn test_restore_statement_with_hlc() {
        let stmt = RestoreStatement::new(
            NodeReference::path("/articles/my-article"),
            RevisionRef::hlc("1734567890123_42"),
        );
        assert_eq!(
            stmt.to_string(),
            "RESTORE NODE path='/articles/my-article' TO REVISION 1734567890123_42"
        );
    }

    #[test]
    fn test_restore_statement_operation() {
        let restore_single =
            RestoreStatement::new(NodeReference::path("/a"), RevisionRef::head_relative(1));
        assert_eq!(restore_single.operation(), "RESTORE NODE");

        let restore_tree =
            RestoreStatement::new_tree(NodeReference::path("/a"), RevisionRef::head_relative(1));
        assert_eq!(restore_tree.operation(), "RESTORE TREE NODE");
    }
}
