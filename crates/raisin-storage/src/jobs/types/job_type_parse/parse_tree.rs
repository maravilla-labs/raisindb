// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parsers for tree snapshot and tree operation job types

use raisin_hlc::HLC;

use super::super::job_type::JobType;

pub(crate) fn parse_tree_snapshot(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("TreeSnapshot(") {
        if let Some(rev_str) = rest.strip_suffix(')') {
            let ts = rev_str
                .parse::<u64>()
                .map_err(|_| format!("Invalid revision number: {}", rev_str))?;
            return Ok(Some(JobType::TreeSnapshot {
                revision: HLC::new(ts, 0),
            }));
        }
    }
    Ok(None)
}

pub(crate) fn parse_tree_operations(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("RevisionHistoryCopy(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 3 {
                let ts = p[2]
                    .parse::<u64>()
                    .map_err(|_| format!("Invalid revision timestamp: {}", p[2]))?;
                return Ok(Some(JobType::RevisionHistoryCopy {
                    source_branch: p[0].to_string(),
                    target_branch: p[1].to_string(),
                    up_to_revision: HLC::new(ts, 0),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("CopyTree(") {
        if let Some(c) = rest.strip_suffix(')') {
            let recursive = c.ends_with('R');
            let c = if recursive { &c[..c.len() - 1] } else { c };
            let p: Vec<&str> = c.split('/').collect();
            match p.len() {
                2 => {
                    return Ok(Some(JobType::CopyTree {
                        source_id: p[0].to_string(),
                        target_parent_id: p[1].to_string(),
                        new_name: None,
                        recursive,
                    }))
                }
                3 => {
                    return Ok(Some(JobType::CopyTree {
                        source_id: p[0].to_string(),
                        target_parent_id: p[1].to_string(),
                        new_name: Some(p[2].to_string()),
                        recursive,
                    }))
                }
                _ => {}
            }
        }
    }
    if let Some(rest) = s.strip_prefix("RestoreTree(") {
        if let Some(c) = rest.strip_suffix(')') {
            let mut recursive = false;
            let (c, translations) = if let Some(idx) = c.find("/T:") {
                let ts = &c[idx + 3..];
                let locales: Vec<String> = ts.split(',').map(|s| s.to_string()).collect();
                (&c[..idx], Some(locales))
            } else {
                (c, None)
            };
            let c = if let Some(stripped) = c.strip_suffix('R') {
                recursive = true;
                stripped
            } else {
                c
            };
            let p: Vec<&str> = c.split('/').collect();
            if p.len() >= 3 {
                return Ok(Some(JobType::RestoreTree {
                    node_id: p[0].to_string(),
                    node_path: p[1..p.len() - 1].join("/"),
                    revision_hlc: p[p.len() - 1].to_string(),
                    recursive,
                    translations,
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("NodeDeleteCleanup(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::NodeDeleteCleanup {
                    node_id: p[0].to_string(),
                    workspace: p[1].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("RelationConsistencyCheck(repair=") {
        if let Some(rs) = rest.strip_suffix(')') {
            return Ok(Some(JobType::RelationConsistencyCheck {
                repair: rs == "true",
            }));
        }
    }
    Ok(None)
}
