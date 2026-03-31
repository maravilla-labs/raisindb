// TODO(v0.2): Fractional indexing utilities
#![allow(dead_code)]

//! Fractional indexing for ordered sequences
//!
//! This module provides a thin wrapper around the `fractional_index` crate,
//! which implements a battle-tested fractional indexing algorithm with guaranteed
//! lexicographic ordering.
//!
//! Fractional indexing allows O(1) insertion between any two items without
//! renumbering siblings. Labels are variable-length strings that maintain
//! their insertion order when sorted lexicographically.
//!
//! # Key Features
//!
//! - **Correct lexicographic ordering**: Uses a proper alphabet that matches ASCII ordering
//! - **No collisions**: Mathematically proven to avoid duplicate labels
//! - **Variable-length labels**: Efficient growth for sequential appends
//! - **Battle-tested**: Used in production systems (Jamsocket, Figma-inspired)
//!
//! # Algorithm
//!
//! The underlying `fractional_index` crate uses a base-95 encoding with proper
//! lexicographic properties. Unlike naive implementations, it correctly handles:
//! - Incrementing: Always produces a larger key
//! - Decrementing: Always produces a smaller key
//! - Midpoint: Always produces a key between two others

mod ops;
#[cfg(test)]
mod tests;

pub use ops::*;
