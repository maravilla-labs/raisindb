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

//! FromStr and TryFrom implementations for JobType
//!
//! Parsing is split by domain:
//! - `parse_indexing` - fulltext, embedding, and index build variants
//! - `parse_tree` - tree snapshot and tree operations
//! - `parse_jobs` - replication, asset processing, HuggingFace, bulk SQL
//! - `parse_functions` - function execution, flow execution, AI
//! - `parse_admin` - package, auth, upload, and custom variants

mod parse_admin;
mod parse_functions;
mod parse_indexing;
mod parse_integrations;
mod parse_jobs;
mod parse_tree;

use super::job_type::JobType;

use parse_admin::{
    parse_auth_variants, parse_custom, parse_package_variants, parse_upload_variants,
};
use parse_functions::{parse_ai_variants, parse_flow_variants, parse_function_variants};
use parse_indexing::{
    parse_embedding_variants, parse_fulltext_variants, parse_index_build_variants,
};
use parse_integrations::parse_integration_variants;
use parse_jobs::{
    parse_asset_processing, parse_bulk_sql, parse_huggingface_variants, parse_replication_variants,
};
use parse_tree::{parse_tree_operations, parse_tree_snapshot};

impl From<JobType> for String {
    fn from(job_type: JobType) -> Self {
        job_type.to_string()
    }
}

impl std::str::FromStr for JobType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "IntegrityScan" => return Ok(Self::IntegrityScan),
            "IndexRebuild" => return Ok(Self::IndexRebuild),
            "IndexVerify" => return Ok(Self::IndexVerify),
            "Compaction" => return Ok(Self::Compaction),
            "Backup" => return Ok(Self::Backup),
            "Restore" => return Ok(Self::Restore),
            "OrphanCleanup" => return Ok(Self::OrphanCleanup),
            "Repair" => return Ok(Self::Repair),
            "FulltextVerify" => return Ok(Self::FulltextVerify),
            "FulltextRebuild" => return Ok(Self::FulltextRebuild),
            "FulltextOptimize" => return Ok(Self::FulltextOptimize),
            "FulltextPurge" => return Ok(Self::FulltextPurge),
            "VectorVerify" => return Ok(Self::VectorVerify),
            "VectorRebuild" => return Ok(Self::VectorRebuild),
            "VectorOptimize" => return Ok(Self::VectorOptimize),
            "VectorRestore" => return Ok(Self::VectorRestore),
            _ => {}
        }
        if let Some(r) = parse_tree_snapshot(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_fulltext_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_embedding_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_huggingface_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_asset_processing(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_replication_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_index_build_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_bulk_sql(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_tree_operations(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_function_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_flow_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_ai_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_package_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_auth_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_upload_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_integration_variants(s)? {
            return Ok(r);
        }
        if let Some(r) = parse_custom(s)? {
            return Ok(r);
        }
        Err(format!("Unknown job type: {}", s))
    }
}

impl TryFrom<String> for JobType {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::JobType;

    fn round_trip(jt: JobType) {
        let s: String = jt.clone().into();
        let parsed: JobType = s.clone().try_into().expect("parse failed");
        assert_eq!(jt, parsed, "round-trip mismatch via {s}");
    }

    /// A `VirtualMountSync` persisted BEFORE `trigger` existed must still parse.
    ///
    /// This matters on the deploy itself, not in theory: the job queue is
    /// durable, so an upgrade lands on a queue that may already hold jobs in the
    /// two-part `mount_id/mode` form. Rejecting them would strand queued syncs
    /// with a parse error at exactly the moment the fix is meant to unstick them.
    #[test]
    fn legacy_two_part_sync_job_still_parses() {
        let parsed: JobType = "VirtualMountSync(mount-123/delta)"
            .to_string()
            .try_into()
            .expect("pre-trigger job form must still parse");
        assert_eq!(
            parsed,
            JobType::VirtualMountSync {
                mount_id: "mount-123".to_string(),
                mode: "delta".to_string(),
                trigger: "unknown".to_string(),
            }
        );
    }

    #[test]
    fn virtual_mount_and_integration_round_trip() {
        round_trip(JobType::VirtualMountSyncCheck {
            tenant_id: Some("acme".to_string()),
            repo_id: Some("docs".to_string()),
        });
        round_trip(JobType::VirtualMountSyncCheck {
            tenant_id: None,
            repo_id: None,
        });
        round_trip(JobType::VirtualMountSync {
            mount_id: "mount-123".to_string(),
            mode: "delta".to_string(),
            trigger: "push".to_string(),
        });
        round_trip(JobType::VirtualMountSync {
            mount_id: "mount-123".to_string(),
            mode: "full".to_string(),
            trigger: "manual".to_string(),
        });
        round_trip(JobType::VirtualMountWriteDrain {
            mount_id: "mount-123".to_string(),
            trigger: "capture".to_string(),
        });
        // The queue is durable, so a persisted drain has to survive a restart.
        // Split from the RIGHT, hence a mount id containing the separator still
        // round-trips rather than silently becoming a different job.
        round_trip(JobType::VirtualMountWriteDrain {
            mount_id: "weird/id".to_string(),
            trigger: "manual".to_string(),
        });
        round_trip(JobType::IntegrationTokenRefresh {
            tenant_id: Some("acme".to_string()),
        });
        round_trip(JobType::IntegrationTokenRefresh { tenant_id: None });
        round_trip(JobType::VirtualMountSubscriptionRenew {
            tenant_id: Some("acme".to_string()),
        });
        round_trip(JobType::VirtualMountSubscriptionRenew { tenant_id: None });
    }

    #[test]
    fn scheduled_invocation_round_trip() {
        round_trip(JobType::ScheduledInvocation {
            invocation_id: "V1StGXR8_Z5jdHi6B-myT".to_string(),
            target_kind: "function".to_string(),
        });
        round_trip(JobType::ScheduledInvocation {
            invocation_id: "abc123".to_string(),
            target_kind: "flow".to_string(),
        });
    }
}
