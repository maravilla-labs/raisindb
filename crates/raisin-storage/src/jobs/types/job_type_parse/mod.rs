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
