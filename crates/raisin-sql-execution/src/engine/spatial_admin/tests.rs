//! Unit tests for the pure parts of spatial admin execution.
//!
//! The statement execution itself needs a real backend and is proven end to end in
//! `crates/raisin-server/tests/all/spatial_admin_test.rs`. What is worth testing in
//! isolation is the settings merge and the scoped-edit semantics, because those
//! decide what lands in a *replicated* record — a mistake there propagates to every
//! peer before anyone notices.

use super::*;
use raisin_models::nodes::properties::{SpatialPropertySchema, SpatialWorkspaceSchema};
use raisin_sql::ast::spatial_admin::SpatialIndexSettings;

#[test]
fn merge_sets_only_the_named_fields() {
    let existing = SpatialPropertySchema {
        precisions: Some(vec![8, 6]),
        srid: Some(3857),
        bucket_property: Some("floor".into()),
        cover: Some(SpatialCoverMode::Extent),
    };
    let settings = SpatialIndexSettings {
        precisions: Some(vec![2, 4, 6]),
        ..Default::default()
    };
    let merged = merge_settings(existing, &settings);

    // Precisions replaced and canonicalised finest-first...
    assert_eq!(merged.precisions, Some(vec![6, 4, 2]));
    // ...everything else untouched, so a one-field ALTER is not a silent reset.
    assert_eq!(merged.srid, Some(3857));
    assert_eq!(merged.bucket_property.as_deref(), Some("floor"));
    assert_eq!(merged.cover, Some(SpatialCoverMode::Extent));
}

#[test]
fn merge_canonicalises_so_the_policy_hash_is_order_independent() {
    let a = merge_settings(
        SpatialPropertySchema::default(),
        &SpatialIndexSettings {
            precisions: Some(vec![2, 6, 4]),
            ..Default::default()
        },
    );
    let b = merge_settings(
        SpatialPropertySchema::default(),
        &SpatialIndexSettings {
            precisions: Some(vec![6, 4, 2]),
            ..Default::default()
        },
    );
    assert_eq!(a, b, "two spellings of one set must not produce two hashes");
}

#[test]
fn scoped_edit_targets_default_or_override() {
    let schema = SpatialWorkspaceSchema::edited(
        None,
        None,
        Some(SpatialPropertySchema {
            precisions: Some(vec![6]),
            ..Default::default()
        }),
    )
    .expect("a non-inert edit must produce a schema");
    assert_eq!(schema.default.precisions, Some(vec![6]));
    assert!(schema.properties.is_empty());

    let schema = SpatialWorkspaceSchema::edited(
        Some(schema),
        Some("location"),
        Some(SpatialPropertySchema {
            precisions: Some(vec![11, 9]),
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(schema.default.precisions, Some(vec![6]));
    assert_eq!(
        schema.properties.get("location").unwrap().precisions,
        Some(vec![11, 9])
    );
}

#[test]
fn reset_of_the_last_scope_removes_the_block_entirely() {
    let schema = SpatialWorkspaceSchema::edited(
        None,
        Some("location"),
        Some(SpatialPropertySchema {
            precisions: Some(vec![6]),
            ..Default::default()
        }),
    );
    assert!(schema.is_some());
    // Resetting the only declaration must leave no empty husk on the workspace
    // record, or `SHOW CONFIG` would keep reporting a declaration that declares
    // nothing.
    assert_eq!(
        SpatialWorkspaceSchema::edited(schema, Some("location"), None),
        None
    );
}

#[test]
fn precision_formatting_round_trips_into_the_ddl() {
    let text = report::format_precisions(&[11, 9, 6]);
    assert_eq!(text, "(11, 9, 6)");
    let sql = format!("ALTER SPATIAL INDEX FOR 'ws' SET PRECISIONS = {}", text);
    // Deliberately parsed back: the reported form is the form an operator will copy.
    let parsed = raisin_sql::ast::spatial_admin_parser::parse_spatial_admin(&sql)
        .expect("reported precisions must parse back")
        .expect("must be recognised as a spatial admin statement");
    match parsed {
        raisin_sql::ast::spatial_admin::SpatialAdminStatement::Alter { settings, .. } => {
            assert_eq!(settings.precisions, Some(vec![11, 9, 6]));
        }
        other => panic!("unexpected: {:?}", other),
    }
}
