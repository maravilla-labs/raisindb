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

//! Configuration models for the authentication system.
//!
//! Follows the pattern established by `TenantAIConfig` for provider configuration.

pub mod policies;
pub mod provider_config;
pub mod repo_config;
pub mod tenant_config;

#[cfg(test)]
mod tests;

pub use policies::*;
pub use provider_config::*;
pub use repo_config::*;
pub use tenant_config::*;
