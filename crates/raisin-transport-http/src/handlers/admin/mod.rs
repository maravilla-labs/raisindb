// SPDX-License-Identifier: BSL-1.1

//! Admin SPA support endpoints.
//!
//! These endpoints exist to support the admin console SPA bootstrap flow.
//! They are intentionally lightweight, unauthenticated, and tenant-aware via
//! the `x-tenant-id` request header.

pub mod bootstrap;
