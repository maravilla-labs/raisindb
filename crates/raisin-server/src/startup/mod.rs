//! Server startup modules.
//!
//! This module contains the various components needed to start the RaisinDB server:
//! - CLI argument parsing and configuration merging
//! - Storage initialization and migration
//! - Event handler registration
//! - Indexing engine setup
//! - Job system initialization
//! - Replication setup for cluster mode
//! - PostgreSQL wire protocol server setup

pub mod binary;
pub mod cli;
pub mod events;
pub mod indexing;
pub mod jobs;
pub mod pgwire;
pub mod replication;
pub mod storage;

pub use cli::{MergedConfig, ServerConfig};
