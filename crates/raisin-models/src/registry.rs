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

//! Tenant and Deployment Registry Models
//!
//! These models track which tenants and deployments are active in the system
//! and maintain metadata about NodeType initialization status.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Registration information for a tenant
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TenantRegistration {
    /// Unique tenant identifier
    pub tenant_id: String,

    /// When this tenant was first registered
    pub created_at: DateTime<Utc>,

    /// Last time this tenant was accessed
    pub last_seen: DateTime<Utc>,

    /// List of deployment keys for this tenant
    pub deployments: Vec<String>,

    /// Additional metadata (billing tier, features, etc.)
    pub metadata: HashMap<String, String>,
}

impl TenantRegistration {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            tenant_id: tenant_id.into(),
            created_at: now,
            last_seen: now,
            deployments: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

/// Registration information for a specific deployment within a tenant
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeploymentRegistration {
    /// Tenant this deployment belongs to
    pub tenant_id: String,

    /// Deployment key (e.g., "production", "staging", "preview-123")
    pub deployment_key: String,

    /// When this deployment was first registered
    pub created_at: DateTime<Utc>,

    /// Last time this deployment was accessed
    pub last_seen: DateTime<Utc>,

    /// Version hash of NodeTypes currently initialized for this deployment
    pub nodetype_version: Option<String>,

    /// Approximate number of nodes in this deployment
    pub node_count: Option<u64>,
}

impl DeploymentRegistration {
    pub fn new(tenant_id: impl Into<String>, deployment_key: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            tenant_id: tenant_id.into(),
            deployment_key: deployment_key.into(),
            created_at: now,
            last_seen: now,
            nodetype_version: None,
            node_count: None,
        }
    }
}
