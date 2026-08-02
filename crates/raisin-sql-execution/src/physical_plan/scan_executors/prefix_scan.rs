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
use super::node_to_row::{node_to_row, OrderContext};
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
        order_cursor,
        order_descending,
        claims_editorial_order,
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
            order_cursor,
            order_descending,
            claims_editorial_order,
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
            order_cursor.clone(),
            *order_descending,
            *claims_editorial_order,
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

    // Only bound the index seek by `limit` when this scan's own order is the
    // order the query asked for. Otherwise a Sort/TopN sits above us and
    // truncating here would drop rows it still needed to consider.
    //
    // Locale fan-out emits one row per locale per node, so the node-level bound
    // has to be scaled by the number of locales or a multi-locale query would
    // come up short.
    let scan_limit = if claims_editorial_order {
        let locale_count = get_locales_to_use(ctx).len().max(1);
        limit.map(|lim| lim.saturating_mul(locale_count))
    } else {
        None
    };

    tracing::info!(
        "   PrefixScan: prefix='{}', direct_children_only={}, workspace='{}', branch='{}', \
         max_revision={:?}, order_cursor={:?}, order_descending={}, claims_order={}",
        path_prefix,
        direct_children_only,
        workspace,
        branch,
        max_revision,
        order_cursor,
        order_descending,
        claims_editorial_order
    );

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
            //
            // Goes through the editorial-order index so each row carries its
            // `__order` label, and so a `__order > cursor` keyset filter and a
            // LIMIT can be pushed all the way down into the index seek.
            // The ordering index keys root-level children under "/" rather than
            // under a node id.
            let parent_id = if parent_dir == "/" {
                Some("/".to_string())
            } else {
                let parent_path = parent_dir.trim_end_matches('/');
                tracing::debug!("   Fetching parent node at path '{}'...", parent_path);
                match storage
                    .nodes()
                    .get_by_path(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), parent_path, max_revision.as_ref())
                    .await?
                {
                    Some(parent) => Some(parent.id),
                    None => {
                        tracing::warn!("   Parent node not found at path '{}', returning 0 rows", parent_path);
                        None
                    }
                }
            };

            // Paged refill.
            //
            // `scan_limit` is now the query's EXACT limit, not a 200k buffer, so
            // the index seek stops after that many children. But rows can still
            // be dropped ABOVE the seek — RLS denies a node, a node fails to
            // load, `path == "/"` is skipped — and a bounded page would then
            // return short. So when a full page comes back and we are still
            // under the limit, fetch the next one, driving the cursor from the
            // last order label seen. This is the contract documented on
            // `list_by_parent_paged_impl`: page size is not a row count.
            let mut page_cursor: Option<String> = order_cursor
                .as_ref()
                .and_then(|c| c.label())
                .map(|label| label.to_string());
            let mut emitted = 0usize;

            'pages: while parent_id.is_some() {
                let list_options = if let Some(rev) = max_revision.as_ref() {
                    raisin_storage::ListOptions::at_revision(*rev)
                } else {
                    raisin_storage::ListOptions::for_sql()
                };

                let nodes = match parent_id.as_ref() {
                    Some(parent_id) => {
                        tracing::debug!(
                            "   Listing direct children of parent '{}' (cursor={:?}, limit={:?}, descending={})",
                            parent_id, page_cursor, scan_limit, order_descending
                        );
                        storage
                            .nodes()
                            .list_by_parent_page(
                                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                                parent_id,
                                page_cursor.as_deref(),
                                scan_limit,
                                order_descending,
                                list_options,
                            )
                            .await?
                    }
                    None => Vec::new(),
                };

                let fetched = nodes.len();
                tracing::info!("   PrefixScan found {} direct children", fetched);

                if fetched == 0 {
                    break 'pages;
                }

                for (node, order_label) in nodes {
                    if let Some(lim) = limit {
                        if emitted >= lim {
                            tracing::debug!("PrefixScan early termination: reached limit of {}", lim);
                            break 'pages;
                        }
                    }

                    // Advance the cursor for every entry examined, including ones
                    // filtered out below — otherwise a page whose rows are all
                    // denied would re-request the same page forever.
                    page_cursor = Some(order_label.clone());

                    safety_scanned += 1;

                    if safety_scanned > SCAN_COUNT_CEILING {
                        tracing::warn!("PrefixScan count limit reached: {} nodes checked", safety_scanned);
                        break 'pages;
                    }

                    if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                        tracing::warn!("PrefixScan time limit reached: {:?} elapsed, {} nodes checked",
                                       start_time.elapsed(), safety_scanned);
                        break 'pages;
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

                    let order_ctx = OrderContext::label(&order_label);

                    for locale in &locales_to_use {
                        let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                            Some(n) => n,
                            None => continue,
                        };

                        let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale, Some(&order_ctx)).await?;
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

                // Another page is worth fetching only when the query is bounded,
                // we came up short of that bound, and this page was full. A short
                // page means the index is exhausted, so stop regardless.
                match (limit, scan_limit) {
                    (Some(lim), Some(page_size)) if emitted < lim && fetched >= page_size => {
                        tracing::debug!(
                            "PrefixScan refill: {} of {} rows survived filtering, fetching next page",
                            emitted, lim
                        );
                    }
                    _ => break 'pages,
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

                    let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale, None,).await?;
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
                // Paged, document-order walk: each node carries its
                // `__tree_order`, and a `__tree_order > cursor` predicate
                // resumes the walk without re-traversing what came before.
                let nodes = storage
                    .nodes()
                    .scan_descendants_ordered_page(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                        &parent.id,
                        order_cursor.as_ref().and_then(|c| c.tree_order()),
                        // The traversal root is emitted but filtered out below,
                        // so leave room for it when bounding the walk.
                        scan_limit.map(|lim| lim.saturating_add(1)),
                        if let Some(rev) = max_revision.as_ref() {
                            raisin_storage::ListOptions::at_revision(*rev)
                        } else {
                            raisin_storage::ListOptions::for_sql()
                        },
                    )
                    .await?;

                tracing::info!("   PrefixScan found {} nodes in tree order", nodes.len());

                let mut emitted = 0usize;

                for (node, tree_order) in nodes {
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

                    let order_ctx = OrderContext {
                        order_label: None,
                        tree_order: Some(&tree_order),
                    };

                    for locale in &locales_to_use {
                        let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                            Some(n) => n,
                            None => continue,
                        };

                        let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale, Some(&order_ctx)).await?;
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
