//! Copy operations for nodes and trees
//!
//! This module provides functions for:
//! - Copying single nodes (`single.rs`)
//! - Copying entire node trees with all descendants and translations (`tree.rs`)
//! - Copying node sets across branches, preserving ids (`cross_branch.rs`)

mod cross_branch;
mod single;
mod tree;
