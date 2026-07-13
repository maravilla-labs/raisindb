// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Prefix scan executor.
//!
//! Uses the path_index column family to efficiently scan nodes with a specific path prefix.
//! Supports two modes:
//! 1. **Direct Children Only** - Uses fast list_by_parent or list_root API
//! 2. **All Descendants** - Uses ORDERED_CHILDREN traversal for tree-ordered results

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::time::Instant;

/// Execute a PrefixScan operator.
///
/// Uses the path_index column family to efficiently scan nodes with a specific path prefix.
/// This is optimal for hierarchical queries like: `WHERE PATH_STARTS_WITH(path, '/content/')`
///
/// # Performance Notes
///
/// - **Direct Children Only** (`direct_children_only=true`):
///   O(k) where k = number of direct children
/// - **All Descendants** (`direct_children_only=false`):
///   O(k) where k = number of matching nodes (tree-ordered traversal)
pub async fn execute_prefix_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (
        tenant_id,
        repo_id,
        branch,
        workspace,
        table,
        alias,
        path_prefix,
        projection,
        direct_children_only,
        limit,
    ) = match plan {
        PhysicalPlan::PrefixScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            path_prefix,
            projection,
            direct_children_only,
            limit,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            path_prefix.clone(),
            projection.clone(),
            *direct_children_only,
            *limit,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for prefix scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let max_revision = ctx.max_revision;
    let ctx_clone = ctx.clone();

    tracing::info!("   PrefixScan: prefix='{}', direct_children_only={}, workspace='{}', branch='{}', max_revision={:?}",
        path_prefix, direct_children_only, workspace, branch, max_revision);

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);
        let start_time = Instant::now();
        let mut safety_scanned = 0usize;

        // The prefix is a raw STRING prefix over paths. It only denotes a parent
        // directory when it ends with '/'; otherwise the final segment is a NAME
        // fragment (e.g. '/job/t-' from `path LIKE '/job/t-%'`) and the parent
        // directory is everything up to the last '/'. Traversal-based scans below
        // operate on the containing directory — a SUPERSET of the prefix match —
        // and rely on the planner's residual PATH_STARTS_WITH filter to narrow
        // it back down.
        let is_directory_prefix = path_prefix.ends_with('/');
        let parent_dir: String = if is_directory_prefix {
            path_prefix.clone()
        } else {
            match path_prefix.rfind('/') {
                Some(idx) => path_prefix[..=idx].to_string(),
                None => "/".to_string(),
            }
        };

        if direct_children_only {
            // FAST PATH: Direct children only (PARENT optimization)
            let nodes = if parent_dir == "/" {
                tracing::debug!("   Listing root-level nodes (direct children of '/')...");
                storage
                    .nodes()
                    .list_root(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        if let Some(rev) = max_revision.as_ref() {
                            raisin_storage::ListOptions::at_revision(*rev)
                        } else {
                            raisin_storage::ListOptions::for_sql()
                        },
                    )
                    .await?
            } else {
                let parent_path = parent_dir.trim_end_matches('/');
                tracing::debug!("   Fetching parent node at path '{}'...", parent_path);

                let parent_node = storage
                    .nodes()
                    .get_by_path(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), parent_path, max_revision.as_ref())
                    .await?;

                if let Some(parent) = parent_node {
                    tracing::debug!("   Listing direct children of parent node '{}'...", parent.id);
                    storage
                        .nodes()
                        .list_by_parent(
                            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                            &parent.id,
                            if let Some(rev) = max_revision.as_ref() {
                                raisin_storage::ListOptions::at_revision(*rev)
                            } else {
                                raisin_storage::ListOptions::for_sql()
                            },
                        )
                        .await?
                } else {
                    tracing::warn!("   Parent node not found at path '{}', returning 0 rows", parent_path);
                    Vec::new()
                }
            };

            tracing::info!("   PrefixScan found {} direct children", nodes.len());

            let mut emitted = 0usize;

            for node in nodes {
                if let Some(lim) = limit {
                    if emitted >= lim {
                        tracing::debug!("PrefixScan early termination: reached limit of {}", lim);
                        break;
                    }
                }

                safety_scanned += 1;

                if safety_scanned > SCAN_COUNT_CEILING {
                    tracing::warn!("PrefixScan count limit reached: {} nodes checked", safety_scanned);
                    break;
                }

                if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                    tracing::warn!("PrefixScan time limit reached: {:?} elapsed, {} nodes checked",
                                   start_time.elapsed(), safety_scanned);
                    break;
                }

                if node.path == "/" {
                    continue;
                }

                // Apply RLS filtering
                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, max_revision.as_ref()).await {
                        Some(n) => n,
                        None => continue,
                    }
                } else {
                    node
                };

                for locale in &locales_to_use {
                    let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };

                    let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;
                    yield row;
                    emitted += 1;
                    if let Some(lim) = limit {
                        if emitted >= lim {
                            tracing::debug!("PrefixScan early termination: reached limit of {}", lim);
                            break;
                        }
                    }
                }
            }
        } else if !is_directory_prefix {
            // NAME-FRAGMENT PREFIX (e.g. '/job/t-'): there is no parent node at
            // the prefix itself, so traversal cannot be anchored there. Use the
            // path index directly — it matches by raw string prefix, which is
            // exactly the `path LIKE 'prefix%'` semantics (and naturally
            // excludes non-matching siblings and the containing directory).
            tracing::debug!("   Scanning path index by string prefix '{}'...", path_prefix);

            let nodes = storage
                .nodes()
                .scan_by_path_prefix(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    &path_prefix,
                    if let Some(rev) = max_revision.as_ref() {
                        raisin_storage::ListOptions::at_revision(*rev)
                    } else {
                        raisin_storage::ListOptions::for_sql()
                    },
                )
                .await?;

            tracing::info!("   PrefixScan found {} nodes by string prefix", nodes.len());

            let mut emitted = 0usize;

            for node in nodes {
                if let Some(lim) = limit {
                    if emitted >= lim {
                        tracing::debug!("PrefixScan early termination: reached limit of {}", lim);
                        break;
                    }
                }

                safety_scanned += 1;

                if safety_scanned > SCAN_COUNT_CEILING {
                    tracing::warn!("PrefixScan count limit reached: {} nodes checked", safety_scanned);
                    break;
                }

                if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                    tracing::warn!("PrefixScan time limit reached: {:?} elapsed, {} nodes checked",
                                   start_time.elapsed(), safety_scanned);
                    break;
                }

                if node.path == "/" {
                    continue;
                }

                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, max_revision.as_ref()).await {
                        Some(n) => n,
                        None => continue,
                    }
                } else {
                    node
                };

                for locale in &locales_to_use {
                    let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };

                    let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;
                    yield row;
                    emitted += 1;
                    if let Some(lim) = limit {
                        if emitted >= lim {
                            tracing::debug!("PrefixScan early termination: reached limit of {}", lim);
                            break;
                        }
                    }
                }
            }
        } else {
            // ORDERED PATH: All descendants using ORDERED_CHILDREN traversal
            tracing::debug!("   Scanning descendants in tree order for path '{}'...", path_prefix);

            let parent_path = path_prefix.trim_end_matches('/');
            let parent_node = storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                    parent_path,
                    if let Some(rev) = max_revision.as_ref() {
                        Some(rev)
                    } else {
                        None
                    },
                )
                .await?;

            if let Some(parent) = parent_node {
                let parent_id = parent.id.clone();
                let nodes = storage
                    .nodes()
                    .scan_descendants_ordered(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        &parent.id,
                        if let Some(rev) = max_revision.as_ref() {
                            raisin_storage::ListOptions::at_revision(*rev)
                        } else {
                            raisin_storage::ListOptions::for_sql()
                        },
                    )
                    .await?;

                tracing::info!("   PrefixScan found {} nodes in tree order", nodes.len());

                let mut emitted = 0usize;

                for node in nodes {
                    // The traversal yields its ROOT too, but the root's path
                    // does not start with '{parent_path}/', so it is NOT part
                    // of the prefix match (`path LIKE '/parent/%'` must not
                    // return '/parent' itself).
                    if node.id == parent_id {
                        continue;
                    }
                    if let Some(lim) = limit {
                        if emitted >= lim {
                            tracing::debug!("PrefixScan early termination: reached limit of {}", lim);
                            break;
                        }
                    }

                    safety_scanned += 1;

                    if safety_scanned > SCAN_COUNT_CEILING {
                        tracing::warn!("PrefixScan count limit reached: {} nodes checked", safety_scanned);
                        break;
                    }

                    if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                        tracing::warn!("PrefixScan time limit reached: {:?} elapsed, {} nodes checked",
                                       start_time.elapsed(), safety_scanned);
                        break;
                    }

                    if node.path == "/" {
                        continue;
                    }

                    let node = if let Some(ref auth) = ctx_clone.auth_context {
                        let scope = PermissionScope::new(&workspace, &branch);
                        match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, max_revision.as_ref()).await {
                            Some(n) => n,
                            None => continue,
                        }
                    } else {
                        node
                    };

                    for locale in &locales_to_use {
                        let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                            Some(n) => n,
                            None => continue,
                        };

                        let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;
                        yield row;
                        emitted += 1;
                        if let Some(lim) = limit {
                            if emitted >= lim {
                                tracing::debug!("PrefixScan early termination: reached limit of {}", lim);
                                break;
                            }
                        }
                    }
                }
            } else {
                tracing::warn!("   Parent node not found at path '{}', returning 0 rows", parent_path);
            }
        }
    }))
}
