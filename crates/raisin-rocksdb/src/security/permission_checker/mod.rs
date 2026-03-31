// TODO(v0.2): Permission checking helpers for RLS enforcement
#![allow(dead_code)]

//! Permission checking for RLS enforcement.
//!
//! This module provides the main entry point for permission checking:
//! - Check if a user can read a node
//! - Check if a user can perform an operation on a node
//! - Filter nodes based on permissions

mod checker;
#[cfg(test)]
mod tests;

pub use checker::{can_read_in_path, PermissionChecker};
