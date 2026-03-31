// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parsers for replication, asset processing, HuggingFace, and bulk SQL job types

use super::super::asset_processing::AssetProcessingOptions;
use super::super::job_type::JobType;

pub(crate) fn parse_asset_processing(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("AssetProcessing(") {
        if let Some(content) = rest.strip_suffix(')') {
            let parts: Vec<&str> = content.split(", ").collect();
            if !parts.is_empty() {
                let node_id = parts[0].to_string();
                let mut options = AssetProcessingOptions::default();
                for part in parts.iter().skip(1) {
                    if let Some(val) = part.strip_prefix("pdf=") {
                        options.extract_pdf_text = val == "true";
                    } else if let Some(val) = part.strip_prefix("img_embed=") {
                        options.generate_image_embedding = val == "true";
                    } else if let Some(val) = part.strip_prefix("caption=") {
                        options.generate_image_caption = val == "true";
                    }
                }
                return Ok(Some(JobType::AssetProcessing { node_id, options }));
            }
        }
    }
    Ok(None)
}

pub(crate) fn parse_replication_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("ReplicationGC(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::ReplicationGC {
                    tenant_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("ReplicationSync(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::ReplicationSync {
                    tenant_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                    peer_id: None,
                }));
            } else if p.len() == 3 {
                return Ok(Some(JobType::ReplicationSync {
                    tenant_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                    peer_id: Some(p[2].to_string()),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("OpLogCompaction(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::OpLogCompaction {
                    tenant_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                }));
            }
        }
    }
    Ok(None)
}

pub(crate) fn parse_huggingface_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("HuggingFaceModelDownload(") {
        if let Some(mid) = rest.strip_suffix(')') {
            return Ok(Some(JobType::HuggingFaceModelDownload {
                model_id: mid.to_string(),
            }));
        }
    }
    if let Some(rest) = s.strip_prefix("HuggingFaceModelDelete(") {
        if let Some(mid) = rest.strip_suffix(')') {
            return Ok(Some(JobType::HuggingFaceModelDelete {
                model_id: mid.to_string(),
            }));
        }
    }
    Ok(None)
}

pub(crate) fn parse_bulk_sql(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("BulkSql(") {
        if let Some(c) = rest.strip_suffix(')') {
            if let Some((actor, _)) = c.split_once(", ") {
                return Ok(Some(JobType::BulkSql {
                    sql: String::new(),
                    actor: actor.to_string(),
                }));
            }
        }
    }
    Ok(None)
}
