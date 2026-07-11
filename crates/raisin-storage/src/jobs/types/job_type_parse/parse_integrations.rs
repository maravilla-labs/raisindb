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

//! Parsers for virtual mount and integration job types

use super::super::job_type::JobType;

/// Parse `VirtualMountSyncCheck`, `VirtualMountSync`, and
/// `IntegrationTokenRefresh` string forms back into a [`JobType`].
pub(crate) fn parse_integration_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("VirtualMountSyncCheck(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                let tid = if p[0] == "*" {
                    None
                } else {
                    Some(p[0].to_string())
                };
                let rid = if p[1] == "*" {
                    None
                } else {
                    Some(p[1].to_string())
                };
                return Ok(Some(JobType::VirtualMountSyncCheck {
                    tenant_id: tid,
                    repo_id: rid,
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("VirtualMountSync(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::VirtualMountSync {
                    mount_id: p[0].to_string(),
                    mode: p[1].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("IntegrationTokenRefresh(") {
        if let Some(c) = rest.strip_suffix(')') {
            let tid = if c == "*" { None } else { Some(c.to_string()) };
            return Ok(Some(JobType::IntegrationTokenRefresh { tenant_id: tid }));
        }
    }
    if let Some(rest) = s.strip_prefix("VirtualMountSubscriptionRenew(") {
        if let Some(c) = rest.strip_suffix(')') {
            let tid = if c == "*" { None } else { Some(c.to_string()) };
            return Ok(Some(JobType::VirtualMountSubscriptionRenew {
                tenant_id: tid,
            }));
        }
    }
    Ok(None)
}
