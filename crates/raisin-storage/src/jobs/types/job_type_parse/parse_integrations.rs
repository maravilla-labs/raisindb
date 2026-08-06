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
    if let Some(rest) = s.strip_prefix("CalendarOccurrenceRebuild(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::CalendarOccurrenceRebuild {
                    tenant_id: (p[0] != "*").then(|| p[0].to_string()),
                    repo_id: (p[1] != "*").then(|| p[1].to_string()),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("VirtualMountWriteReconcile(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::VirtualMountWriteReconcile {
                    tenant_id: (p[0] != "*").then(|| p[0].to_string()),
                    repo_id: (p[1] != "*").then(|| p[1].to_string()),
                }));
            }
        }
    }
    // Split from the RIGHT: `trigger` is a closed set of slugs and never
    // contains a `/`, whereas a mount id is a node id and this parser has no
    // business assuming it does not.
    if let Some(rest) = s.strip_prefix("VirtualMountWriteDrain(") {
        if let Some(c) = rest.strip_suffix(')') {
            if let Some((mount_id, trigger)) = c.rsplit_once('/') {
                return Ok(Some(JobType::VirtualMountWriteDrain {
                    mount_id: mount_id.to_string(),
                    trigger: trigger.to_string(),
                }));
            }
        }
    }
    // NOTE: must stay BELOW the `VirtualMountWriteReconcile` arm only if that
    // prefix were a prefix of this one; it is not, but the `VirtualMountSync(`
    // prefix IS a prefix of nothing else, so order here is free.
    if let Some(rest) = s.strip_prefix("VirtualMountSync(") {
        if let Some(c) = rest.strip_suffix(')') {
            // 2 parts is the pre-`trigger` form; still accepted so jobs
            // persisted before the field existed keep parsing.
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 || p.len() == 3 {
                return Ok(Some(JobType::VirtualMountSync {
                    mount_id: p[0].to_string(),
                    mode: p[1].to_string(),
                    trigger: p
                        .get(2)
                        .map_or_else(|| "unknown".to_string(), |t| t.to_string()),
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
