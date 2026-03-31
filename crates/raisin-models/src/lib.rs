// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

// TODO(v0.2): Clean up unused code and ambiguous re-exports
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(ambiguous_glob_reexports)]
// ! Data models and type definitions for RaisinDB.
//!
//! This crate contains all the core data structures used throughout RaisinDB:
//!
//! - [`nodes`] - Node, NodeType, and property-related models
//! - [`translations`] - Multi-language translation support
//! - [`workspace`] - Workspace configuration and metadata
//! - [`registry`] - Multi-tenant deployment and tenant registration
//! - [`errors`] - Error types specific to model validation
//!
//! # Core Concepts
//!
//! ## Nodes
//!
//! Nodes are the primary content entities, organized in a hierarchical tree
//! structure within workspaces. Each node has a type (NodeType) that defines
//! its schema and behavior.
//!
//! ## Workspaces
//!
//! Workspaces are isolated containers for content. Each workspace has its own
//! set of nodes, node types, and configuration.
//!
//! ## Properties
//!
//! Nodes store data in a flexible property system that supports nested objects,
//! arrays, and various primitive types, all validated against schemas.

pub mod admin_user;
pub mod api_key;
pub mod auth;
pub mod errors;
pub mod fractional_index;
pub mod migrations;
pub mod nodes;
pub mod operations;
pub mod permissions;
pub mod registry;
pub mod timestamp;
pub mod translations;
pub mod tree;
pub mod workspace;

pub use errors::*;
pub use timestamp::StorageTimestamp;
pub use tree::*;
pub use workspace::*;
