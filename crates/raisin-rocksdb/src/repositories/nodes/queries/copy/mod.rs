//! Copy operations for nodes and trees
//!
//! This module provides functions for:
//! - Copying single nodes (`single.rs`)
//! - Copying entire node trees with all descendants and translations (`tree.rs`)
//! - Copying node sets across branches, preserving ids (`cross_branch.rs`)
//!
//! Secrets split along the id: a copy MINTS a new node id so it re-vaults
//! (`revault.rs`), while a cross-branch promotion PRESERVES the id so it copies
//! the sealed record into the target branch (`cross_branch/secrets.rs`).

mod cross_branch;
mod revault;
mod single;
mod tree;
