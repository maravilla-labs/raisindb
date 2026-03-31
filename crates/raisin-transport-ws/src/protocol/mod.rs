// SPDX-License-Identifier: BSL-1.1

//! WebSocket protocol definitions
//!
//! This module defines the message envelope structures and types for the RaisinDB WebSocket protocol.
//! All messages are serialized using MessagePack for efficiency.
//!
//! ## Submodules
//!
//! - [`envelopes`] -- Request/response/event envelopes, context, metadata, and the
//!   [`RequestType`] enum.
//! - [`payloads_node`] -- Payloads for node CRUD, manipulation, tree, property,
//!   and relationship operations.
//! - [`payloads_schema`] -- Payloads for node-type, archetype, and element-type
//!   management.
//! - [`payloads_ops`] -- Payloads for branches, tags, workspaces, repositories,
//!   translations, transactions, subscriptions, and authentication.

mod envelopes;
mod payloads_node;
mod payloads_ops;
mod payloads_schema;

// Re-export everything so that `crate::protocol::Foo` keeps working.
pub use envelopes::*;
pub use payloads_node::*;
pub use payloads_ops::*;
pub use payloads_schema::*;
