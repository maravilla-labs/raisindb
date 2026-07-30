// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Spatial scan executors.
//!
//! Finds nodes using geospatial queries via the spatial index.
//! - `SpatialDistanceScan` - nodes within a given radius of a point
//! - `SpatialKnnScan` - k nearest neighbors to a point

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::PermissionScope;
use raisin_storage::spatial::SpatialPreFilter;
use raisin_storage::{NodeRepository, SpatialIndexRepository, Storage, StorageScope};

/// Execute a SpatialDistanceScan operator.
///
/// Finds nodes within a given distance of a point using the spatial index.
/// Uses geohash-based indexing for efficient proximity queries.
///
/// # Performance
/// - Uses geohash cell expansion for candidate filtering
/// - Post-filters with exact Haversine distance calculation
pub async fn execute_spatial_distance_scan<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
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
        property_name,
        center_lon,
        center_lat,
        radius_meters,
        projection,
        limit,
        claims_distance_order,
        precisions,
        bucket_eq,
        fallback,
    ) = match plan {
        PhysicalPlan::SpatialDistanceScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            property_name,
            center_lon,
            center_lat,
            radius_meters,
            projection,
            limit,
            claims_distance_order,
            precisions,
            bucket_eq,
            fallback,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            property_name.clone(),
            *center_lon,
            *center_lat,
            *radius_meters,
            projection.clone(),
            *limit,
            *claims_distance_order,
            precisions.clone(),
            bucket_eq.clone(),
            fallback.as_deref(),
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for spatial distance scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone();
    let max_revision = ctx.max_revision.unwrap_or_else(raisin_hlc::HLC::now);
    // No artificial default — return all haversine-filtered results, same as
    // PostGIS. The SQL LIMIT clause (when present) is enforced separately via
    // the `emitted` counter in the stream below.
    let scan_limit = limit.unwrap_or(usize::MAX);

    // `claims_distance_order` does not change what this executor DOES — the index
    // returns ascending distance order unconditionally. It records that the
    // PLANNER relied on that order to drop a `Sort`, so it is logged: if rows ever
    // come back mis-ordered, the trace says whether a Sort was elided on the
    // strength of a promise this scan no longer keeps.
    tracing::info!(
        "   SpatialDistanceScan: property='{}', center=({}, {}), radius={}m, workspace='{}', branch='{}', limit={:?}, claims_distance_order={}",
        property_name, center_lon, center_lat, radius_meters, workspace, branch, limit, claims_distance_order
    );

    // The discriminator pre-filter is a SELECTIVITY device only: it rejects
    // candidates whose stored bucket provably differs, and never rejects an entry
    // that carries no bucket. The predicate it came from is still in the residual
    // filter above this scan, so a stale bucket costs a wasted node fetch, never a
    // dropped row.
    let prefilter = SpatialPreFilter {
        bucket_eq: bucket_eq.map(|(_, value)| value),
        bbox: None,
    };

    // Resolved BEFORE the stream is built, because a per-cell budget exhaustion
    // has to be answered by running a DIFFERENT plan, and that decision cannot be
    // made from inside the stream this function returns.
    let results = match storage.spatial_index().find_within_radius(
        &tenant_id,
        &repo_id,
        &branch,
        &workspace,
        &property_name,
        center_lon,
        center_lat,
        radius_meters,
        &max_revision,
        scan_limit,
        &precisions,
        &prefilter,
    ) {
        Ok(results) => results,
        // DEGRADE, do not fail. The index cannot answer this query without
        // answering short, and a short spatial answer is the one outcome this
        // subsystem refuses — so run the fallback the planner attached, which
        // re-applies every predicate per row and is therefore exact.
        Err(error) if error.is_spatial_budget_exceeded() => {
            let Some(fallback) = fallback else {
                return Err(error);
            };
            tracing::warn!(
                workspace = %workspace,
                property = %property_name,
                reason = %error,
                "Spatial index cell budget exhausted; DEGRADING to a full row scan for this                  query. Results stay correct and the scan is slow. This is superseded-revision                  accumulation on a high-frequency property — reduce its precision set, or let                  the spatial compaction filter prune (see docs/OPEN-ITEMS.md 2.99/2.100)."
            );
            return crate::physical_plan::executor::execute_plan(fallback, ctx).await;
        }
        Err(error) => return Err(error),
    };

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);

        tracing::info!("   SpatialDistanceScan found {} nodes within {}m", results.len(), radius_meters);

        let mut emitted = 0;

        for proximity_result in results {
            if let Some(lim) = limit {
                if emitted >= lim { break; }
            }

            let node_opt = storage
                .nodes()
                .get(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &proximity_result.node_id, None)
                .await?;

            if let Some(node) = node_opt {
                if node.path == "/" { continue; }

                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, Some(&max_revision)).await {
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

                    let mut row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale, None,).await?;

                    row.insert(
                        "__distance".to_string(),
                        PropertyValue::Float(proximity_result.distance_meters),
                    );
                    // Which geometry field produced that distance. On this path it
                    // is trivially the property the scan was planned for, but a
                    // node can now carry several geometries at several depths, so
                    // "which one matched" has to be answerable — and answerable the
                    // same way whichever field was named.
                    row.insert(
                        "__matched_path".to_string(),
                        PropertyValue::String(property_name.clone()),
                    );

                    yield row;
                    emitted += 1;
                }
            }
        }
    }))
}

/// Execute a SpatialKnnScan operator.
///
/// Finds k nearest neighbors to a point using the spatial index.
/// Uses progressive ring expansion for efficient k-NN queries.
///
/// # Performance
/// - Starts at high precision and expands outward
/// - Adaptive based on data density
pub async fn execute_spatial_knn_scan<S: Storage + 'static>(
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
        property_name,
        center_lon,
        center_lat,
        k,
        projection,
        precisions,
    ) = match plan {
        PhysicalPlan::SpatialKnnScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            property_name,
            center_lon,
            center_lat,
            k,
            projection,
            claims_distance_order: _,
            precisions,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            property_name.clone(),
            *center_lon,
            *center_lat,
            *k,
            projection.clone(),
            precisions.clone(),
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for spatial knn scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone();
    let max_revision = ctx.max_revision.unwrap_or_else(raisin_hlc::HLC::now);

    tracing::info!(
        "   SpatialKnnScan: property='{}', center=({}, {}), k={}, workspace='{}', branch='{}'",
        property_name,
        center_lon,
        center_lat,
        k,
        workspace,
        branch
    );

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);

        let results = storage
            .spatial_index()
            .find_nearest(
                &tenant_id, &repo_id, &branch, &workspace,
                &property_name, center_lon, center_lat, k,
                &max_revision, &precisions, &SpatialPreFilter::default(),
            )?;

        tracing::info!("   SpatialKnnScan found {} nearest neighbors", results.len());

        let mut emitted = 0;

        for proximity_result in results {
            if emitted >= k { break; }

            let node_opt = storage
                .nodes()
                .get(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &proximity_result.node_id, None)
                .await?;

            if let Some(node) = node_opt {
                if node.path == "/" { continue; }

                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match crate::physical_plan::scan_executors::helpers::rls_filter_node_graph(&*storage, node, auth, &scope, &tenant_id, &repo_id, &branch, Some(&max_revision)).await {
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

                    let mut row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale, None,).await?;

                    row.insert(
                        "__distance".to_string(),
                        PropertyValue::Float(proximity_result.distance_meters),
                    );
                    // Which geometry field produced that distance. On this path it
                    // is trivially the property the scan was planned for, but a
                    // node can now carry several geometries at several depths, so
                    // "which one matched" has to be answerable — and answerable the
                    // same way whichever field was named.
                    row.insert(
                        "__matched_path".to_string(),
                        PropertyValue::String(property_name.clone()),
                    );

                    yield row;
                    emitted += 1;
                }
            }
        }
    }))
}
