// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parsers for fulltext, embedding, and index build job types

use super::super::index_operation::IndexOperation;
use super::super::job_type::JobType;

pub(crate) fn parse_fulltext_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("FulltextIndex(") {
        if let Some(content) = rest.strip_suffix(')') {
            let parts: Vec<&str> = content.split(", ").collect();
            if parts.len() == 2 {
                let op = match parts[1] {
                    "AddOrUpdate" => IndexOperation::AddOrUpdate,
                    "Delete" => IndexOperation::Delete,
                    _ => return Err(format!("Invalid operation: {}", parts[1])),
                };
                return Ok(Some(JobType::FulltextIndex {
                    node_id: parts[0].to_string(),
                    operation: op,
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("FulltextBranchCopy(") {
        if let Some(sb) = rest.strip_suffix(')') {
            return Ok(Some(JobType::FulltextBranchCopy {
                source_branch: sb.to_string(),
            }));
        }
    }
    if let Some(rest) = s.strip_prefix("FulltextBatchIndex(count=") {
        if let Some(cs) = rest.strip_suffix(')') {
            let oc = cs
                .parse::<usize>()
                .map_err(|_| format!("Invalid operation count: {}", cs))?;
            return Ok(Some(JobType::FulltextBatchIndex {
                operation_count: oc,
            }));
        }
    }
    Ok(None)
}

pub(crate) fn parse_embedding_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("EmbeddingGenerate(") {
        if let Some(nid) = rest.strip_suffix(')') {
            return Ok(Some(JobType::EmbeddingGenerate {
                node_id: nid.to_string(),
            }));
        }
    }
    if let Some(rest) = s.strip_prefix("EmbeddingDelete(") {
        if let Some(nid) = rest.strip_suffix(')') {
            return Ok(Some(JobType::EmbeddingDelete {
                node_id: nid.to_string(),
            }));
        }
    }
    if let Some(rest) = s.strip_prefix("EmbeddingBranchCopy(") {
        if let Some(sb) = rest.strip_suffix(')') {
            return Ok(Some(JobType::EmbeddingBranchCopy {
                source_branch: sb.to_string(),
            }));
        }
    }
    Ok(None)
}

pub(crate) fn parse_index_build_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("PropertyIndexBuild(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 4 {
                return Ok(Some(JobType::PropertyIndexBuild {
                    tenant_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                    branch: p[2].to_string(),
                    workspace: p[3].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("CompoundIndexBuild(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 6 {
                return Ok(Some(JobType::CompoundIndexBuild {
                    tenant_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                    branch: p[2].to_string(),
                    workspace: p[3].to_string(),
                    node_type_name: p[4].to_string(),
                    index_name: p[5].to_string(),
                }));
            }
        }
    }
    Ok(None)
}
