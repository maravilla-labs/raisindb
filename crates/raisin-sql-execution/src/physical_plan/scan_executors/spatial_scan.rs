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
use raisin_storage::{NodeRepository, SpatialIndexRepository, Storage, StorageScope};

/// Execute a SpatialDistanceScan operator.
///
/// Finds nodes within a given distance of a point using the spatial index.
/// Uses geohash-based indexing for efficient proximity queries.
///
/// # Performance
/// - Uses geohash cell expansion for candidate filtering
/// - Post-filters with exact Haversine distance calculation
pub async fn execute_spatial_distance_scan<S: Storage + 'static>(
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

    tracing::info!(
        "   SpatialDistanceScan: property='{}', center=({}, {}), radius={}m, workspace='{}', branch='{}', limit={:?}",
        property_name, center_lon, center_lat, radius_meters, workspace, branch, limit
    );

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);

        let results = storage
            .spatial_index()
            .find_within_radius(
                &tenant_id, &repo_id, &branch, &workspace,
                &property_name, center_lon, center_lat, radius_meters,
                &max_revision, scan_limit,
            )?;

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
                    match rls_filter::filter_node(node, auth, &scope) {
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

                    let mut row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;

                    row.insert(
                        "__distance".to_string(),
                        PropertyValue::Float(proximity_result.distance_meters),
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
                &max_revision,
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
                    match rls_filter::filter_node(node, auth, &scope) {
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

                    let mut row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;

                    row.insert(
                        "__distance".to_string(),
                        PropertyValue::Float(proximity_result.distance_meters),
                    );

                    yield row;
                    emitted += 1;
                }
            }
        }
    }))
}
