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

//! Precedence, canonicalization and fingerprint stability for spatial policy.

use super::*;
use crate::nodes::properties::schema::{PropertyType, PropertyValueSchema};

fn prop_schema(spatial: Option<SpatialPropertySchema>) -> PropertyValueSchema {
    PropertyValueSchema {
        name: Some("location".into()),
        property_type: PropertyType::Geometry,
        required: None,
        unique: None,
        default: None,
        constraints: None,
        structure: None,
        items: None,
        value: None,
        meta: None,
        is_translatable: None,
        allow_additional_properties: None,
        index: None,
        spatial,
    }
}

#[test]
fn default_policy_uses_the_shipped_precision_set() {
    let p = SpatialPolicy::default();
    assert_eq!(p.precisions, vec![11, 10, 9, 8, 7, 6, 4, 2]);
    assert_eq!(p.finest(), 11);
    assert_eq!(p.coarsest(), 2);
    assert!(!p.covers_non_point());
}

#[test]
fn no_config_anywhere_yields_the_default() {
    assert_eq!(
        resolve_spatial_policy(None, None, "location"),
        SpatialPolicy::default()
    );
}

#[test]
fn node_type_beats_workspace_property_beats_workspace_default() {
    let mut ws = SpatialWorkspaceSchema {
        default: SpatialPropertySchema {
            precisions: Some(vec![6]),
            ..Default::default()
        },
        properties: Default::default(),
    };
    // Workspace default only.
    assert_eq!(
        resolve_spatial_policy(None, Some(&ws), "location").precisions,
        vec![6]
    );

    // Per-property override wins over the workspace default.
    ws.properties.insert(
        "location".into(),
        SpatialPropertySchema {
            precisions: Some(vec![7, 8]),
            ..Default::default()
        },
    );
    assert_eq!(
        resolve_spatial_policy(None, Some(&ws), "location").precisions,
        vec![8, 7]
    );

    // A NodeType declaration beats both.
    let schema = prop_schema(Some(SpatialPropertySchema {
        precisions: Some(vec![10, 11]),
        ..Default::default()
    }));
    assert_eq!(
        resolve_spatial_policy(Some(&schema), Some(&ws), "location").precisions,
        vec![11, 10]
    );
}

/// The interesting part of the precedence rule: fields resolve *independently*,
/// so a narrow scope that sets one thing still inherits the rest.
#[test]
fn fields_inherit_independently() {
    let ws = SpatialWorkspaceSchema {
        default: SpatialPropertySchema {
            precisions: Some(vec![6]),
            srid: Some(2056),
            bucket_property: Some("floor".into()),
            cover: Some(SpatialCoverMode::Extent),
        },
        properties: Default::default(),
    };
    let schema = prop_schema(Some(SpatialPropertySchema {
        precisions: Some(vec![9]),
        ..Default::default()
    }));

    let policy = resolve_spatial_policy(Some(&schema), Some(&ws), "location");
    assert_eq!(policy.precisions, vec![9], "NodeType wins for precisions");
    assert_eq!(policy.srid, Some(2056), "srid inherited from workspace");
    assert_eq!(policy.bucket_property.as_deref(), Some("floor"));
    assert_eq!(policy.cover, SpatialCoverMode::Extent);
}

#[test]
fn a_property_without_a_spatial_block_inherits_everything() {
    let ws = SpatialWorkspaceSchema {
        default: SpatialPropertySchema {
            bucket_property: Some("level".into()),
            ..Default::default()
        },
        properties: Default::default(),
    };
    let schema = prop_schema(None);
    let policy = resolve_spatial_policy(Some(&schema), Some(&ws), "location");
    assert_eq!(policy.bucket_property.as_deref(), Some("level"));
    assert_eq!(policy.precisions, SpatialPolicy::default().precisions);
}

#[test]
fn precisions_are_canonicalized_not_trusted() {
    // Out of range values dropped, duplicates collapsed, order normalized.
    assert_eq!(sorted_precisions(vec![8, 0, 8, 13, 4, 99]), vec![8, 4]);
    // Nothing valid left => the default, not an empty set (which would index
    // nothing and silently make the property unqueryable).
    assert_eq!(
        sorted_precisions(vec![0, 42]),
        sorted_precisions(INDEX_PRECISIONS_DEFAULT.to_vec())
    );
}

#[test]
fn policy_hash_ignores_the_order_it_was_written_in() {
    let a = resolve_spatial_policy(
        Some(&prop_schema(Some(SpatialPropertySchema {
            precisions: Some(vec![7, 9, 11]),
            ..Default::default()
        }))),
        None,
        "location",
    );
    let b = resolve_spatial_policy(
        Some(&prop_schema(Some(SpatialPropertySchema {
            precisions: Some(vec![11, 7, 9, 9]),
            ..Default::default()
        }))),
        None,
        "location",
    );
    assert_eq!(a.policy_hash(), b.policy_hash());
}

#[test]
fn policy_hash_changes_when_anything_material_changes() {
    let base = SpatialPolicy::default();
    let h = base.policy_hash();

    let mut p = base.clone();
    p.precisions = vec![8];
    assert_ne!(p.policy_hash(), h, "precision set");

    let mut p = base.clone();
    p.srid = Some(3857);
    assert_ne!(p.policy_hash(), h, "srid");

    let mut p = base.clone();
    p.bucket_property = Some("floor".into());
    assert_ne!(p.policy_hash(), h, "bucket property");

    let mut p = base.clone();
    p.cover = SpatialCoverMode::Extent;
    assert_ne!(p.policy_hash(), h, "cover mode");
}

/// A persisted fingerprint must not drift, so pin the literal. If this fails you
/// have changed the on-disk contract: every workspace will reindex. That may be
/// intended (bump `SPATIAL_NORMALIZER_VERSION` and update the literal), but it
/// must never happen by accident.
#[test]
fn default_policy_hash_is_pinned() {
    assert_eq!(
        SpatialPolicy::default().policy_hash(),
        0xf181_a570_f4ff_293b
    );
}

#[test]
fn schema_round_trips_through_json() {
    let s = SpatialWorkspaceSchema {
        default: SpatialPropertySchema {
            precisions: Some(vec![11, 8]),
            srid: Some(2056),
            bucket_property: Some("floor".into()),
            cover: Some(SpatialCoverMode::Extent),
        },
        properties: [(
            "other".to_string(),
            SpatialPropertySchema {
                precisions: Some(vec![6]),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
    };
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"cover\":\"extent\""), "{json}");
    let back: SpatialWorkspaceSchema = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
}

/// An absent block must serialize to nothing at all, so adding this field does
/// not rewrite every stored NodeType.
#[test]
fn an_empty_property_schema_serializes_to_an_empty_object() {
    let s = SpatialPropertySchema::default();
    assert_eq!(serde_json::to_string(&s).unwrap(), "{}");
}
