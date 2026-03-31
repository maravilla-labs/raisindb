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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    pub node_id: String,
    pub path: String,
    pub workspace: String,
    pub user_id: Option<String>,
    pub action: AuditLogAction,
    pub timestamp: DateTime<Utc>,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditLogAction {
    Create,
    Update,
    UpdateProperty,
    Delete,
    Read,
    Viewed,
    Publish,
    Unpublish,
    Share,
    Unshare,
    Move,
    Copy,
    Rename,
    Restore,
    DeleteVersion,
}
