//! Spatial planning tests.
//!
//! THE INVARIANT under test: *a predicate may be removed from the residual filter
//! ONLY IF the chosen access path is a proven-complete, exact answer for it.* In
//! every other case the predicate stays and the query is slower but correct. A
//! spatial query must never return fewer rows than the truth.
//!
//! It used to be violated in the way that produces silent wrong answers:
//! `has_spatial_index()` returned a hardcoded `true` ("the spatial_index CF is
//! always present" — a statement about schema, not about whether anything was
//! ever indexed), and `build_spatial_scan` stripped `SpatialDWithin` from the
//! residual filter on the strength of it. On an unpopulated or stale index the
//! query returned ZERO ROWS with no fallback and no warning.

use super::*;
use crate::physical_plan::catalog::{SpatialAvailability, SpatialStateSource};
use crate::physical_plan::operators::ScanReason;
use raisin_sql::analyzer::functions::{FunctionCategory, FunctionSignature};
use raisin_sql::logical_plan::{FilterPredicate, SortExpr, TableSchema};
use std::sync::Arc;

// ── harness ────────────────────────────────────────────────────────────────

/// A state source that reports one fixed answer for every property.
#[derive(Debug)]
struct FixedState(SpatialAvailability);

impl SpatialStateSource for FixedState {
    fn spatial_availability(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _property: &str,
    ) -> SpatialAvailability {
        self.0.clone()
    }
}

fn ready(precisions: Vec<usize>) -> SpatialAvailability {
    SpatialAvailability::Ready {
        precisions,
        built_through: raisin_hlc::HLC::now(),
        bucket_property: Some("floor".to_string()),
    }
}

/// A planner whose spatial index reports `availability`.
fn planner_with(availability: SpatialAvailability) -> PhysicalPlanner {
    let catalog = crate::physical_plan::catalog::RocksDBIndexCatalog::new()
        .with_spatial_state(Arc::new(FixedState(availability)));
    PhysicalPlanner::with_catalog(
        "default".into(),
        "default".into(),
        "main".into(),
        "shops".into(),
        Arc::new(catalog),
    )
}

/// A planner with the default catalog — i.e. no spatial state source at all,
/// which is what every existing construction site produces.
fn planner_without_state() -> PhysicalPlanner {
    PhysicalPlanner::with_context(
        "default".into(),
        "default".into(),
        "main".into(),
        "shops".into(),
    )
}

fn scan() -> LogicalPlan {
    LogicalPlan::Scan {
        table: "nodes".to_string(),
        alias: None,
        schema: Arc::new(TableSchema {
            table_name: "nodes".to_string(),
            columns: vec![],
        }),
        filter: None,
        projection: None,
        workspace: Some("shops".to_string()),
        max_revision: None,
        branch_override: None,
        locales: vec![],
    }
}

fn func(name: &str, args: Vec<TypedExpr>, ret: DataType) -> TypedExpr {
    TypedExpr::new(
        Expr::Function {
            name: name.to_string(),
            args,
            signature: FunctionSignature {
                name: name.to_string(),
                params: vec![],
                return_type: ret.clone(),
                is_deterministic: true,
                category: FunctionCategory::Geospatial,
            },
            filter: None,
        },
        ret,
    )
}

/// `properties->>'location'` — the geometry source spelling users actually write.
fn geom_source(property: &str) -> TypedExpr {
    TypedExpr::new(
        Expr::JsonExtractText {
            object: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "properties".to_string(),
                DataType::JsonB,
            )),
            key: Box::new(TypedExpr::literal(Literal::Text(property.to_string()))),
        },
        DataType::Text,
    )
}

fn st_point(lon: f64, lat: f64) -> TypedExpr {
    func(
        "ST_POINT",
        vec![
            TypedExpr::literal(Literal::Double(lon)),
            TypedExpr::literal(Literal::Double(lat)),
        ],
        DataType::Geometry,
    )
}

fn dwithin(property: &str, lon: f64, lat: f64, radius: f64) -> TypedExpr {
    func(
        "ST_DWITHIN",
        vec![
            geom_source(property),
            st_point(lon, lat),
            TypedExpr::literal(Literal::Double(radius)),
        ],
        DataType::Boolean,
    )
}

fn filtered(predicate: TypedExpr) -> LogicalPlan {
    LogicalPlan::Filter {
        input: Box::new(scan()),
        predicate: FilterPredicate::from_expr(predicate),
    }
}

/// Descend through `Filter`/`Project`/`Limit`/`Sort` wrappers to the leaf scan.
fn leaf(plan: &PhysicalPlan) -> &PhysicalPlan {
    let mut node = plan;
    loop {
        match node {
            PhysicalPlan::Filter { input, .. }
            | PhysicalPlan::Project { input, .. }
            | PhysicalPlan::Limit { input, .. }
            | PhysicalPlan::TopN { input, .. }
            | PhysicalPlan::Sort { input, .. } => node = input,
            other => return other,
        }
    }
}

/// Whether the plan re-applies an `ST_*` predicate above the scan.
fn has_spatial_residual_filter(plan: &PhysicalPlan) -> bool {
    match plan {
        PhysicalPlan::Filter { predicates, .. } => predicates.iter().any(mentions_spatial_fn),
        PhysicalPlan::TableScan { filter, .. } => filter.as_ref().is_some_and(mentions_spatial_fn),
        PhysicalPlan::Project { input, .. }
        | PhysicalPlan::Limit { input, .. }
        | PhysicalPlan::Sort { input, .. }
        | PhysicalPlan::TopN { input, .. } => has_spatial_residual_filter(input),
        _ => false,
    }
}

fn mentions_spatial_fn(expr: &TypedExpr) -> bool {
    match &expr.expr {
        Expr::Function { name, args, .. } => {
            name.to_uppercase().starts_with("ST_") || args.iter().any(mentions_spatial_fn)
        }
        Expr::BinaryOp { left, right, .. } => {
            mentions_spatial_fn(left) || mentions_spatial_fn(right)
        }
        Expr::Cast { expr, .. } => mentions_spatial_fn(expr),
        _ => false,
    }
}

// ── the silent-empty trap ──────────────────────────────────────────────────

/// The regression. With no spatial state source (which is every call site that
/// has not been wired up), the planner must NOT pick a spatial scan, and the
/// `ST_DWITHIN` must survive as a row-level filter.
#[test]
fn unbuilt_index_falls_back_to_a_scan_that_still_filters() {
    let planner = planner_without_state();
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 500.0)))
        .unwrap();

    assert!(
        !matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. }),
        "an unbuilt spatial index must not drive the scan: {}",
        plan.describe()
    );
    assert!(
        has_spatial_residual_filter(&plan),
        "ST_DWITHIN must survive as a row-level filter when the index cannot \
         answer it, or the query silently returns zero rows: {}",
        plan.describe()
    );
}

/// The fallback must be *visible*, naming the workspace, the property and the
/// remedy — not a generic "no matching index".
#[test]
fn unbuilt_index_fallback_is_annotated_for_explain() {
    let planner = planner_with(SpatialAvailability::NotBuilt);
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 500.0)))
        .unwrap();

    let PhysicalPlan::TableScan { reason, .. } = leaf(&plan) else {
        panic!("expected a TableScan fallback, got {}", plan.describe());
    };
    let ScanReason::SpatialIndexUnusable {
        workspace,
        property,
        detail,
    } = reason
    else {
        panic!("expected SpatialIndexUnusable, got {}", reason);
    };
    assert_eq!(workspace, "shops");
    assert_eq!(property, "location");
    assert!(
        detail.contains("REBUILD SPATIAL INDEX"),
        "detail: {}",
        detail
    );
    assert!(reason.to_string().contains("applied per row"));
}

/// An `Unusable` state record is not the same as a missing one, and EXPLAIN says
/// which — but both refuse the index.
#[test]
fn unusable_index_state_also_refuses_the_index() {
    let planner = planner_with(SpatialAvailability::Unusable("bad bytes".into()));
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 500.0)))
        .unwrap();
    assert!(!matches!(
        leaf(&plan),
        PhysicalPlan::SpatialDistanceScan { .. }
    ));
    assert!(has_spatial_residual_filter(&plan));
    assert!(format!("{}", plan.describe()).contains("TableScan"));
}

/// A `Ready` index DOES drive the scan, and only then may the predicate be
/// dropped.
#[test]
fn ready_index_drives_the_scan_and_may_strip_the_predicate() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 500.0)))
        .unwrap();

    assert!(
        matches!(&plan, PhysicalPlan::SpatialDistanceScan { .. }),
        "expected a bare SpatialDistanceScan, got {}",
        plan.describe()
    );
    assert!(!has_spatial_residual_filter(&plan));
}

/// Sub-metre and continental radii must both work. The old fixed precision set
/// plus a `\0`-terminated geohash prefix meant anything outside roughly
/// 4.8 m - 39 km silently returned nothing.
#[test]
fn the_whole_radius_range_is_planned_against_the_index() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    for radius in [0.5, 5.0, 50.0, 500.0, 5_000.0, 50_000.0, 500_000.0] {
        let plan = planner
            .plan(&filtered(dwithin("location", 8.54, 47.37, radius)))
            .unwrap();
        assert!(
            matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. }),
            "radius {}m should still use the spatial index, got {}",
            radius,
            plan.describe()
        );
    }
}

/// A radius no configured precision can cover within the cell budget is a
/// coverage failure, not a licence to return a partial answer.
#[test]
fn an_uncoverable_radius_keeps_the_predicate() {
    // precision 8 only => 38 m cells => a 100 km radius needs ~2632 rings.
    let planner = planner_with(ready(vec![8]));
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 100_000.0)))
        .unwrap();
    assert!(!matches!(
        leaf(&plan),
        PhysicalPlan::SpatialDistanceScan { .. }
    ));
    assert!(has_spatial_residual_filter(&plan));
}

// ── widened predicate shapes ───────────────────────────────────────────────

/// Reversed argument order is the same predicate and must reach the index.
#[test]
fn reversed_argument_order_is_recognised() {
    let planner = planner_with(ready(vec![8, 6]));
    let reversed = func(
        "ST_DWITHIN",
        vec![
            st_point(8.54, 47.37),
            geom_source("location"),
            TypedExpr::literal(Literal::Int(500)),
        ],
        DataType::Boolean,
    );
    let plan = planner.plan(&filtered(reversed)).unwrap();
    let PhysicalPlan::SpatialDistanceScan {
        center_lon,
        center_lat,
        radius_meters,
        ..
    } = leaf(&plan)
    else {
        panic!("expected SpatialDistanceScan, got {}", plan.describe());
    };
    assert_eq!(*center_lon, 8.54);
    assert_eq!(*center_lat, 47.37);
    assert_eq!(*radius_meters, 500.0);
}

/// `ST_DISTANCE(g, c) <= r` denotes the same access path. `<` is a STRICT bound
/// while the scan's post-filter is `<=`, so that spelling must keep the predicate.
#[test]
fn st_distance_bounds_become_a_distance_scan_and_only_the_inclusive_form_strips() {
    let planner = planner_with(ready(vec![8, 6]));

    for (op, expect_strip) in [(BinaryOperator::LtEq, true), (BinaryOperator::Lt, false)] {
        let distance = func(
            "ST_DISTANCE",
            vec![geom_source("location"), st_point(8.54, 47.37)],
            DataType::Double,
        );
        let predicate = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(distance),
                op,
                right: Box::new(TypedExpr::literal(Literal::Double(500.0))),
            },
            DataType::Boolean,
        );
        let plan = planner.plan(&filtered(predicate)).unwrap();
        assert!(
            matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. }),
            "ST_DISTANCE bound should use the index, got {}",
            plan.describe()
        );
        assert_eq!(
            !has_spatial_residual_filter(&plan),
            expect_strip,
            "strictness handling wrong for {:?}: {}",
            op,
            plan.describe()
        );
    }
}

/// `ST_DISTANCE(...) > r` is an anti-range with no index path. It must fall back
/// to a scan with the predicate intact, never be silently dropped.
#[test]
fn st_distance_lower_bounds_have_no_index_path_and_keep_the_predicate() {
    let planner = planner_with(ready(vec![8, 6]));
    let distance = func(
        "ST_DISTANCE",
        vec![geom_source("location"), st_point(8.54, 47.37)],
        DataType::Double,
    );
    let predicate = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(distance),
            op: BinaryOperator::Gt,
            right: Box::new(TypedExpr::literal(Literal::Double(500.0))),
        },
        DataType::Boolean,
    );
    let plan = planner.plan(&filtered(predicate)).unwrap();
    assert!(!matches!(
        leaf(&plan),
        PhysicalPlan::SpatialDistanceScan { .. }
    ));
    assert!(has_spatial_residual_filter(&plan));
}

/// A non-point centre is reduced to its envelope centre with the radius inflated,
/// which is a strict WIDENING — so the scan may be used for candidates but the
/// original predicate must be re-applied.
#[test]
fn a_polygon_centre_widens_the_window_and_never_strips() {
    let planner = planner_with(ready(vec![8, 6, 4]));
    let polygon = TypedExpr::literal(Literal::Geometry(serde_json::json!({
        "type": "Polygon",
        "coordinates": [[[8.5, 47.3], [8.6, 47.3], [8.6, 47.4], [8.5, 47.4], [8.5, 47.3]]],
    })));
    let predicate = func(
        "ST_DWITHIN",
        vec![
            geom_source("location"),
            polygon,
            TypedExpr::literal(Literal::Double(100.0)),
        ],
        DataType::Boolean,
    );
    let plan = planner.plan(&filtered(predicate)).unwrap();
    let PhysicalPlan::SpatialDistanceScan { radius_meters, .. } = leaf(&plan) else {
        panic!("expected SpatialDistanceScan, got {}", plan.describe());
    };
    assert!(
        *radius_meters > 100.0,
        "radius must be inflated by the envelope circumradius, got {}",
        radius_meters
    );
    assert!(
        has_spatial_residual_filter(&plan),
        "a widened window is candidates-only: {}",
        plan.describe()
    );
}

/// Integer literals are numbers too. `ST_DWITHIN(loc, ST_POINT(8, 47), 500)`
/// used to decline the index purely because the literals were not `Double`.
#[test]
fn integer_literals_are_accepted() {
    let planner = planner_with(ready(vec![8, 6]));
    let predicate = func(
        "ST_DWITHIN",
        vec![
            geom_source("location"),
            func(
                "ST_POINT",
                vec![
                    TypedExpr::literal(Literal::Int(8)),
                    TypedExpr::literal(Literal::Int(47)),
                ],
                DataType::Geometry,
            ),
            TypedExpr::literal(Literal::Int(500)),
        ],
        DataType::Boolean,
    );
    let plan = planner.plan(&filtered(predicate)).unwrap();
    assert!(matches!(
        leaf(&plan),
        PhysicalPlan::SpatialDistanceScan { .. }
    ));
}

// ── composition ───────────────────────────────────────────────────────────

/// Spatial must compose with `node_type =`, a property predicate (the floor /
/// level filter) and `DESCENDANT_OF`: whichever access path wins, the other
/// predicates stay as row-level filters. Dropping one of them is the
/// `path LIKE '/a/%' AND node_type = 'X'` bug all over again.
#[test]
fn spatial_composes_with_node_type_floor_and_hierarchy() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));

    let node_type_eq = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "node_type".to_string(),
                DataType::Text,
            )),
            op: BinaryOperator::Eq,
            right: Box::new(TypedExpr::literal(Literal::Text("Shop".into()))),
        },
        DataType::Boolean,
    );
    let floor_eq = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(geom_source("floor")),
            op: BinaryOperator::Eq,
            right: Box::new(TypedExpr::literal(Literal::Text("L2".into()))),
        },
        DataType::Boolean,
    );
    let descendant = func(
        "DESCENDANT_OF",
        vec![TypedExpr::literal(Literal::Path("/terminal-a".into()))],
        DataType::Boolean,
    );

    let all = [
        dwithin("location", 8.54, 47.37, 50.0),
        node_type_eq,
        floor_eq,
        descendant,
    ]
    .into_iter()
    .reduce(|acc, next| {
        TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(acc),
                op: BinaryOperator::And,
                right: Box::new(next),
            },
            DataType::Boolean,
        )
    })
    .unwrap();

    let plan = planner.plan(&filtered(all)).unwrap();
    let rendered = format!("{:?}", plan);

    // Whatever won the access path, none of the four may vanish.
    assert!(rendered.contains("DESCENDANT_OF"), "lost DESCENDANT_OF");
    assert!(rendered.contains("L2"), "lost the floor predicate");
    assert!(rendered.contains("Shop"), "lost node_type = 'Shop'");
    // The spatial predicate is either the access path or a residual filter.
    let spatial_is_access_path = matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. });
    assert!(
        spatial_is_access_path || has_spatial_residual_filter(&plan),
        "the spatial predicate is neither an access path nor a filter: {}",
        plan.describe()
    );
}

// ── LIMIT pushdown ────────────────────────────────────────────────────────

/// A residual filter above a bounded scan returns SHORT: the scan truncates to
/// `limit` candidates and the filter then discards some of them. The limit must
/// stay above the filter.
#[test]
fn limit_is_not_pushed_below_a_residual_filter() {
    let planner = planner_with(ready(vec![8, 6]));
    let node_type_eq = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "node_type".to_string(),
                DataType::Text,
            )),
            op: BinaryOperator::Eq,
            right: Box::new(TypedExpr::literal(Literal::Text("Shop".into()))),
        },
        DataType::Boolean,
    );
    let predicate = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(dwithin("location", 8.54, 47.37, 50.0)),
            op: BinaryOperator::And,
            right: Box::new(node_type_eq),
        },
        DataType::Boolean,
    );
    let plan = planner
        .plan(&LogicalPlan::Limit {
            input: Box::new(filtered(predicate)),
            limit: 5,
            offset: 0,
        })
        .unwrap();

    // Find any spatial scan in the tree and assert it is unbounded.
    fn spatial_limit(plan: &PhysicalPlan) -> Option<Option<usize>> {
        if let PhysicalPlan::SpatialDistanceScan { limit, .. } = plan {
            return Some(*limit);
        }
        plan.inputs().iter().find_map(|p| spatial_limit(p))
    }
    if let Some(limit) = spatial_limit(&plan) {
        assert_eq!(
            limit,
            None,
            "a bounded spatial scan under a residual filter returns short: {}",
            plan.describe()
        );
    }
}

/// With no residual filter and no ORDER BY, bounding the scan in its own
/// (distance) order is a valid answer for `... LIMIT k`.
#[test]
fn limit_is_pushed_when_nothing_can_filter_or_reorder() {
    let planner = planner_with(ready(vec![8, 6]));
    let plan = planner
        .plan(&LogicalPlan::Limit {
            input: Box::new(filtered(dwithin("location", 8.54, 47.37, 50.0))),
            limit: 5,
            offset: 0,
        })
        .unwrap();
    fn spatial_limit(plan: &PhysicalPlan) -> Option<Option<usize>> {
        if let PhysicalPlan::SpatialDistanceScan { limit, .. } = plan {
            return Some(*limit);
        }
        plan.inputs().iter().find_map(|p| spatial_limit(p))
    }
    assert_eq!(spatial_limit(&plan), Some(Some(5)));
}

// ── k-NN and sort elision ─────────────────────────────────────────────────

/// `ORDER BY ST_DISTANCE(...) LIMIT k` becomes a `SpatialKnnScan`.
/// `SpatialKnnScan` was previously dead code: the variant, the executor and the
/// storage method all existed and no planner site ever built one.
#[test]
fn order_by_st_distance_limit_k_becomes_a_knn_scan() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let sort = LogicalPlan::Sort {
        input: Box::new(scan()),
        sort_exprs: vec![SortExpr {
            expr: func(
                "ST_DISTANCE",
                vec![geom_source("location"), st_point(8.54, 47.37)],
                DataType::Double,
            ),
            ascending: true,
            nulls_first: false,
        }],
    };
    let plan = planner
        .plan(&LogicalPlan::Limit {
            input: Box::new(sort),
            limit: 10,
            offset: 0,
        })
        .unwrap();

    let PhysicalPlan::SpatialKnnScan {
        property_name,
        center_lon,
        k,
        claims_distance_order,
        ..
    } = leaf(&plan)
    else {
        panic!("expected SpatialKnnScan, got {}", plan.describe());
    };
    assert_eq!(property_name, "location");
    assert_eq!(*center_lon, 8.54);
    assert_eq!(*k, 10);
    assert!(*claims_distance_order);
    // No Sort / TopN survives above it.
    assert!(!format!("{:?}", plan).contains("TopN"));
}

/// The k-NN scan must keep the `Project` it looked through.
///
/// A scan emits fully qualified column names (`nodes.name`, see `node_to_row`'s
/// "Column Naming" section) and the `Project` above it is what turns those into
/// the names the SELECT list asked for. `try_plan_spatial_knn` originally
/// returned the bare `SpatialKnnScan`, which produced the right ROWS under the
/// wrong COLUMN NAMES — `SELECT name ... ORDER BY ST_DISTANCE(...) LIMIT k` came
/// back with a `nodes.name` column and no `name`, which every client reads as a
/// null. The `Sort` and the `Limit` are what this scan replaces; the projection
/// is not.
#[test]
fn knn_keeps_the_projection_it_looked_through() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let project = LogicalPlan::Project {
        input: Box::new(scan()),
        exprs: vec![ProjectionExpr {
            expr: TypedExpr::new(
                Expr::Column {
                    table: "nodes".to_string(),
                    column: "name".to_string(),
                },
                DataType::Text,
            ),
            alias: "name".to_string(),
        }],
    };
    let sort = LogicalPlan::Sort {
        input: Box::new(project),
        sort_exprs: vec![SortExpr {
            expr: func(
                "ST_DISTANCE",
                vec![geom_source("location"), st_point(8.54, 47.37)],
                DataType::Double,
            ),
            ascending: true,
            nulls_first: false,
        }],
    };
    let plan = planner
        .plan(&LogicalPlan::Limit {
            input: Box::new(sort),
            limit: 3,
            offset: 0,
        })
        .unwrap();

    // The scan is still chosen ...
    assert!(matches!(leaf(&plan), PhysicalPlan::SpatialKnnScan { .. }));
    // ... and the SELECT list is still on top of it.
    let PhysicalPlan::Project { exprs, .. } = &plan else {
        panic!(
            "expected the Project to survive above the k-NN scan, got {}",
            plan.describe()
        );
    };
    assert_eq!(exprs.len(), 1);
    assert_eq!(exprs[0].alias, "name");
}

/// DESC has no bounded access path (the index walks nearest-first), so it must
/// fall through to TopN rather than silently returning the *nearest* k.
#[test]
fn descending_distance_order_is_not_turned_into_knn() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let sort = LogicalPlan::Sort {
        input: Box::new(scan()),
        sort_exprs: vec![SortExpr {
            expr: func(
                "ST_DISTANCE",
                vec![geom_source("location"), st_point(8.54, 47.37)],
                DataType::Double,
            ),
            ascending: false,
            nulls_first: false,
        }],
    };
    let plan = planner
        .plan(&LogicalPlan::Limit {
            input: Box::new(sort),
            limit: 10,
            offset: 0,
        })
        .unwrap();
    assert!(!matches!(leaf(&plan), PhysicalPlan::SpatialKnnScan { .. }));
}

/// An unbuilt index must not be asked for the nearest anything either.
#[test]
fn knn_also_fails_closed_on_an_unbuilt_index() {
    let planner = planner_with(SpatialAvailability::NotBuilt);
    let sort = LogicalPlan::Sort {
        input: Box::new(scan()),
        sort_exprs: vec![SortExpr {
            expr: func(
                "ST_DISTANCE",
                vec![geom_source("location"), st_point(8.54, 47.37)],
                DataType::Double,
            ),
            ascending: true,
            nulls_first: false,
        }],
    };
    let plan = planner
        .plan(&LogicalPlan::Limit {
            input: Box::new(sort),
            limit: 10,
            offset: 0,
        })
        .unwrap();
    assert!(!matches!(leaf(&plan), PhysicalPlan::SpatialKnnScan { .. }));
}

/// `WHERE ST_DWITHIN(...) ORDER BY ST_DISTANCE(same centre) ASC` — the scan
/// already emits that order, so the Sort is elided and the flag that says so is
/// actually READ (the `CompoundIndexScan.pre_sorted` mistake was to set it and
/// never consult it).
#[test]
fn distance_order_over_a_radius_scan_elides_the_sort() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let sort = LogicalPlan::Sort {
        input: Box::new(filtered(dwithin("location", 8.54, 47.37, 500.0))),
        sort_exprs: vec![SortExpr {
            expr: func(
                "ST_DISTANCE",
                vec![geom_source("location"), st_point(8.54, 47.37)],
                DataType::Double,
            ),
            ascending: true,
            nulls_first: false,
        }],
    };
    let plan = planner.plan(&sort).unwrap();
    assert!(
        matches!(
            &plan,
            PhysicalPlan::SpatialDistanceScan {
                claims_distance_order: true,
                ..
            }
        ),
        "expected the Sort to be elided over a distance-ordered scan, got {}",
        plan.describe()
    );
}

/// A DIFFERENT centre is a different order. Eliding there would silently
/// mis-order the result.
#[test]
fn distance_order_about_a_different_centre_is_not_elided() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let sort = LogicalPlan::Sort {
        input: Box::new(filtered(dwithin("location", 8.54, 47.37, 500.0))),
        sort_exprs: vec![SortExpr {
            expr: func(
                "ST_DISTANCE",
                vec![geom_source("location"), st_point(0.0, 0.0)],
                DataType::Double,
            ),
            ascending: true,
            nulls_first: false,
        }],
    };
    let plan = planner.plan(&sort).unwrap();
    assert!(
        matches!(&plan, PhysicalPlan::Sort { .. }),
        "a different centre must keep the Sort, got {}",
        plan.describe()
    );
}
