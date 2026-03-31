// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parsers for package, auth, upload, and custom job types

use super::super::job_type::JobType;

pub(crate) fn parse_package_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("PackageInstall(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::PackageInstall {
                    package_name: p[0].to_string(),
                    package_version: p[1].to_string(),
                    package_node_id: String::new(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("PackageProcess(") {
        if let Some(pnid) = rest.strip_suffix(')') {
            return Ok(Some(JobType::PackageProcess {
                package_node_id: pnid.to_string(),
            }));
        }
    }
    if let Some(rest) = s.strip_prefix("PackageExport(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::PackageExport {
                    package_name: p[0].to_string(),
                    package_node_id: String::new(),
                    export_mode: p[1].to_string(),
                    include_modifications: true,
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("PackageSyncStatus(") {
        if let Some(pnid) = rest.strip_suffix(')') {
            return Ok(Some(JobType::PackageSyncStatus {
                package_node_id: pnid.to_string(),
                compute_hashes: true,
            }));
        }
    }
    if let Some(rest) = s.strip_prefix("PackageSyncPush(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                return Ok(Some(JobType::PackageSyncPush {
                    package_node_id: p[0].to_string(),
                    paths_to_sync: Vec::new(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("PackageSyncPull(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 3 {
                return Ok(Some(JobType::PackageSyncPull {
                    package_node_id: p[0].to_string(),
                    paths_to_pull: Vec::new(),
                    conflict_resolution: p[2].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("PackageCreateFromSelection(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 3 {
                return Ok(Some(JobType::PackageCreateFromSelection {
                    package_name: p[0].to_string(),
                    package_version: p[1].to_string(),
                    include_node_types: p[2] == "types",
                }));
            }
        }
    }
    Ok(None)
}

pub(crate) fn parse_auth_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("AuthMagicLinkSend(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 3 {
                return Ok(Some(JobType::AuthMagicLinkSend {
                    identity_id: p[0].to_string(),
                    email: p[1].to_string(),
                    token_id: p[2].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("AuthSessionCleanup(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                let tid = if p[0] == "*" {
                    None
                } else {
                    Some(p[0].to_string())
                };
                let bs = p[1]
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid batch_size: {}", p[1]))?;
                return Ok(Some(JobType::AuthSessionCleanup {
                    tenant_id: tid,
                    batch_size: bs,
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("AuthTokenCleanup(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 2 {
                let tid = if p[0] == "*" {
                    None
                } else {
                    Some(p[0].to_string())
                };
                let tt = if p[1] == "*" {
                    Vec::new()
                } else {
                    p[1].split(',').map(|s| s.to_string()).collect()
                };
                return Ok(Some(JobType::AuthTokenCleanup {
                    tenant_id: tid,
                    token_types: tt,
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("AuthAccessNotification(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 3 {
                return Ok(Some(JobType::AuthAccessNotification {
                    identity_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                    notification_type: p[2].to_string(),
                }));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("AuthCreateUserNode(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            if p.len() == 3 {
                return Ok(Some(JobType::AuthCreateUserNode {
                    identity_id: p[0].to_string(),
                    repo_id: p[1].to_string(),
                    email: p[2].to_string(),
                    display_name: None,
                    default_roles: Vec::new(),
                }));
            }
        }
    }
    Ok(None)
}

pub(crate) fn parse_upload_variants(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("ResumableUploadComplete(") {
        if let Some(c) = rest.strip_suffix(')') {
            let p: Vec<&str> = c.split('/').collect();
            match p.len() {
                1 => {
                    return Ok(Some(JobType::ResumableUploadComplete {
                        upload_id: p[0].to_string(),
                        commit_message: None,
                        commit_actor: None,
                    }))
                }
                2 => {
                    return Ok(Some(JobType::ResumableUploadComplete {
                        upload_id: p[0].to_string(),
                        commit_message: Some(p[1].to_string()),
                        commit_actor: None,
                    }))
                }
                3 => {
                    return Ok(Some(JobType::ResumableUploadComplete {
                        upload_id: p[0].to_string(),
                        commit_message: Some(p[1].to_string()),
                        commit_actor: Some(p[2].to_string()),
                    }))
                }
                _ => {}
            }
        }
    }
    if let Some(rest) = s.strip_prefix("UploadSessionCleanup(") {
        if let Some(uid) = rest.strip_suffix(')') {
            return Ok(Some(JobType::UploadSessionCleanup {
                upload_id: uid.to_string(),
            }));
        }
    }
    Ok(None)
}

pub(crate) fn parse_custom(s: &str) -> Result<Option<JobType>, String> {
    if let Some(rest) = s.strip_prefix("Custom(") {
        if let Some(name) = rest.strip_suffix(')') {
            return Ok(Some(JobType::Custom(name.to_string())));
        }
    }
    Ok(None)
}
