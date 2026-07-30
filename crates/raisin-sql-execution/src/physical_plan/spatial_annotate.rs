// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Spatial annotation: `__distance` and `__matched_path` on the fallback path.
//!
//! # Why this operator exists
//!
//! `SpatialDistanceScan` reads a candidate's distance off the index entry and
//! injects `__distance` / `__matched_path` itself. The spatial FALLBACK — an
//! ordinary scan with the spatial predicate retained per row — has no index entry
//! to read, and the row-level predicate evaluation throws its distance away the
//! moment it has answered true or false.
//!
//! That is precisely the case where the two columns matter most. A WILDCARD path
//! (`stops[].geo`) can never take the index scan (see `build_spatial_scan`), so
//! every wildcard query lands here — and "which of this node's geometries
//! matched?" is the whole reason the wildcard spelling exists.
//!
//! # Agreement with the predicate
//!
//! The distance is recomputed through the same helper the ST_\* functions use,
//! with the same wildcard rule (the MINIMUM over the matched geometries). A
//! second, independently-written distance here would eventually disagree with the
//! predicate that admitted the row, and a row that reports a `__distance` larger
//! than the radius it was selected by is worse than no column at all.

use crate::physical_plan::eval::functions::geospatial::nearest_geometry;
use crate::physical_plan::executor::{execute_plan, ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use futures::stream::StreamExt;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

/// Execute a `SpatialAnnotate` operator.
///
/// Passes every input row through unchanged, adding `__distance` (metres) and
/// `__matched_path` (the concrete dotted path) when the row's property tree
/// carries a geometry the pattern addresses. A row whose geometry cannot be
/// resolved is emitted untouched, leaving both columns NULL — never dropped: this
/// operator annotates, it does not filter.
pub async fn execute_spatial_annotate<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, property_name, center_lon, center_lat) = match plan {
        PhysicalPlan::SpatialAnnotate {
            input,
            property_name,
            center_lon,
            center_lat,
        } => (
            input.as_ref(),
            property_name.clone(),
            *center_lon,
            *center_lat,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for spatial annotate".to_string(),
            ))
        }
    };

    let mut input_stream = execute_plan(input, ctx).await?;

    Ok(Box::pin(try_stream! {
        while let Some(row_result) = input_stream.next().await {
            let mut row = row_result?;

            let nearest = row
                .get_by_unqualified("properties")
                .and_then(|properties| {
                    nearest_geometry(properties, &property_name, center_lon, center_lat)
                });

            if let Some(nearest) = nearest {
                row.insert(
                    "__distance".to_string(),
                    PropertyValue::Float(nearest.distance_meters),
                );
                row.insert(
                    "__matched_path".to_string(),
                    PropertyValue::String(nearest.path),
                );
            }

            yield row;
        }
    }))
}
