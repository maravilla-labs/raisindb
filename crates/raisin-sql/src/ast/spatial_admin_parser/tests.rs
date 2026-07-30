//! Parser tests for the `SPATIAL INDEX` admin grammar.

use super::*;

#[test]
fn guard_matches_only_the_family() {
    assert!(is_spatial_admin_statement("SHOW SPATIAL INDEX CONFIG"));
    assert!(is_spatial_admin_statement("show spatial index health"));
    assert!(is_spatial_admin_statement(
        "ALTER SPATIAL INDEX FOR 'ws' SET PRECISIONS = (6)"
    ));
    assert!(is_spatial_admin_statement("REBUILD SPATIAL INDEX FOR 'ws'"));
    assert!(is_spatial_admin_statement("VERIFY SPATIAL INDEX FOR 'ws'"));
    // Whitespace is normalised, so a newline between keywords still matches.
    assert!(is_spatial_admin_statement("SHOW\n  SPATIAL\tINDEX HEALTH"));

    assert!(!is_spatial_admin_statement("SHOW VECTOR INDEX HEALTH"));
    assert!(!is_spatial_admin_statement("SELECT 1"));
    assert!(!is_spatial_admin_statement("REBUILD VECTOR INDEX"));
}

#[test]
fn show_without_target_is_workspace_wide() {
    let stmt = parse_spatial_admin("SHOW SPATIAL INDEX CONFIG")
        .unwrap()
        .unwrap();
    assert_eq!(
        stmt,
        SpatialAdminStatement::ShowConfig {
            workspace: None,
            property: None
        }
    );
    assert!(stmt.is_read_only());
}

#[test]
fn show_health_with_property() {
    let stmt = parse_spatial_admin("SHOW SPATIAL INDEX HEALTH FOR 'shops' PROPERTY 'location';")
        .unwrap()
        .unwrap();
    assert_eq!(
        stmt,
        SpatialAdminStatement::ShowHealth {
            workspace: Some("shops".into()),
            property: Some("location".into())
        }
    );
}

#[test]
fn alter_sets_every_field() {
    let stmt = parse_spatial_admin(
        "ALTER SPATIAL INDEX FOR 'shops' PROPERTY 'location' \
         SET PRECISIONS = (11, 10, 9, 8, 7, 6, 4, 2) \
         SET SRID = 4326 SET BUCKET PROPERTY = 'floor' SET COVER = EXTENT",
    )
    .unwrap()
    .unwrap();

    let SpatialAdminStatement::Alter {
        workspace,
        property,
        settings,
    } = stmt
    else {
        panic!("expected Alter");
    };
    assert_eq!(workspace, "shops");
    assert_eq!(property.as_deref(), Some("location"));
    assert_eq!(
        settings.precisions.as_deref(),
        Some(&[11, 10, 9, 8, 7, 6, 4, 2][..])
    );
    assert_eq!(settings.srid, Some(4326));
    assert_eq!(settings.bucket_property.as_deref(), Some("floor"));
    assert_eq!(settings.cover, Some(CoverModeSpec::Extent));
}

#[test]
fn alter_without_property_targets_the_workspace_default() {
    let stmt = parse_spatial_admin("ALTER SPATIAL INDEX FOR 'shops' SET PRECISIONS = (6, 4, 2)")
        .unwrap()
        .unwrap();
    let SpatialAdminStatement::Alter { property, .. } = stmt else {
        panic!("expected Alter");
    };
    assert!(property.is_none());
}

#[test]
fn reset_is_its_own_statement() {
    let stmt = parse_spatial_admin("ALTER SPATIAL INDEX FOR 'shops' PROPERTY 'location' RESET")
        .unwrap()
        .unwrap();
    assert_eq!(
        stmt,
        SpatialAdminStatement::Reset {
            workspace: "shops".into(),
            property: Some("location".into())
        }
    );
    assert!(!stmt.is_read_only());
}

#[test]
fn rebuild_and_verify() {
    assert_eq!(
        parse_spatial_admin("REBUILD SPATIAL INDEX FOR 'shops'")
            .unwrap()
            .unwrap(),
        SpatialAdminStatement::Rebuild {
            workspace: "shops".into(),
            property: None
        }
    );
    assert_eq!(
        parse_spatial_admin("VERIFY SPATIAL INDEX FOR 'shops' PROPERTY 'loc'")
            .unwrap()
            .unwrap(),
        SpatialAdminStatement::Verify {
            workspace: "shops".into(),
            property: Some("loc".into())
        }
    );
    // VERIFY scans the index, so it is deliberately NOT read-only for
    // authorization purposes even though it writes nothing.
    assert!(!parse_spatial_admin("VERIFY SPATIAL INDEX FOR 'shops'")
        .unwrap()
        .unwrap()
        .is_read_only());
}

#[test]
fn alter_with_no_set_clause_is_rejected() {
    let err = parse_spatial_admin("ALTER SPATIAL INDEX FOR 'shops'").unwrap_err();
    assert!(
        err.message.contains("Parse error") || err.message.contains("at least one SET"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn out_of_range_precision_is_rejected_at_parse_time() {
    // Rejected before it can reach a replicated workspace record.
    let err = parse_spatial_admin("ALTER SPATIAL INDEX FOR 'shops' SET PRECISIONS = (2, 40)")
        .unwrap_err();
    assert!(err.message.contains("out of range"), "{}", err.message);
}

#[test]
fn duplicate_precisions_are_rejected() {
    let err =
        parse_spatial_admin("ALTER SPATIAL INDEX FOR 'shops' SET PRECISIONS = (6, 6)").unwrap_err();
    assert!(err.message.contains("duplicates"), "{}", err.message);
}

#[test]
fn a_set_whose_coarsest_cell_is_sub_kilometre_is_rejected() {
    // Precision 7 cells are ~150 m: as the COARSEST entry that cannot answer a
    // city-scale radius inside the cell budget, so every wide query would silently
    // degrade to a full scan.
    let err = parse_spatial_admin("ALTER SPATIAL INDEX FOR 'shops' SET PRECISIONS = (11, 9, 7)")
        .unwrap_err();
    assert!(err.message.contains("too fine"), "{}", err.message);
}

#[test]
fn trailing_junk_is_an_error_not_silence() {
    let err = parse_spatial_admin("REBUILD SPATIAL INDEX FOR 'shops' NOW").unwrap_err();
    assert!(err.message.contains("trailing"), "{}", err.message);
}

#[test]
fn non_matching_sql_returns_none() {
    assert_eq!(parse_spatial_admin("SELECT 1").unwrap(), None);
}

#[test]
fn display_round_trips_the_shape() {
    let stmt = parse_spatial_admin(
        "ALTER SPATIAL INDEX FOR 'shops' PROPERTY 'location' SET PRECISIONS = (6, 4, 2)",
    )
    .unwrap()
    .unwrap();
    let text = stmt.to_string();
    assert!(text.contains("ALTER SPATIAL INDEX"), "{}", text);
    assert!(text.contains("FOR 'shops'"), "{}", text);
    assert!(text.contains("PROPERTY 'location'"), "{}", text);
    assert!(text.contains("SET PRECISIONS = (6, 4, 2)"), "{}", text);
}
