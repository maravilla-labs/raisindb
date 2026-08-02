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

/// Descend through `Filter`/`Project`/`Limit`/`Sort`/`SpatialAnnotate` wrappers
/// to the leaf scan. `SpatialAnnotate` is a pass-through that materialises
/// `__distance` / `__matched_path`; it changes no row and hides no scan.
fn leaf(plan: &PhysicalPlan) -> &PhysicalPlan {
    let mut node = plan;
    loop {
        match node {
            PhysicalPlan::Filter { input, .. }
            | PhysicalPlan::Project { input, .. }
            | PhysicalPlan::Limit { input, .. }
            | PhysicalPlan::TopN { input, .. }
            | PhysicalPlan::SpatialAnnotate { input, .. }
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
        | PhysicalPlan::SpatialAnnotate { input, .. }
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
    assert!(leaf(&plan).describe().contains("TableScan"));
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

// ── nested and wildcard property paths ─────────────────────────────────────

/// A NESTED path is an ordinary property name as far as the planner is
/// concerned: the index key's property segment is the dotted path verbatim, so a
/// `Ready` index for `venue.geo` is index-backed exactly like `location`.
#[test]
fn a_nested_dotted_path_is_index_backed_like_any_other_property() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let plan = planner
        .plan(&filtered(dwithin("venue.geo", 8.54, 47.37, 500.0)))
        .unwrap();
    match leaf(&plan) {
        PhysicalPlan::SpatialDistanceScan { property_name, .. } => {
            assert_eq!(property_name, "venue.geo");
        }
        other => panic!("expected a SpatialDistanceScan, got {}", other.describe()),
    }
}

/// THE trap: a wildcard path must NEVER become a `SpatialDistanceScan`.
///
/// The state record for `stops[].geo` legitimately says `Ready` (array indices
/// normalise onto that one key), but each element's ENTRIES live under its own
/// concrete path — so an index scan over the prefix `stops[].geo` would read a
/// prefix holding nothing and return zero rows. It must take the fallback.
#[test]
fn a_wildcard_path_never_takes_the_index_scan() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let plan = planner
        .plan(&filtered(dwithin("stops[].geo", 8.54, 47.37, 500.0)))
        .unwrap();

    assert!(
        !matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. }),
        "a wildcard path must not scan an index namespace that holds nothing, got {}",
        plan.describe()
    );
    assert!(
        has_spatial_residual_filter(&plan),
        "the predicate must survive as a row-level filter, got {}",
        plan.describe()
    );
    match leaf(&plan) {
        PhysicalPlan::TableScan {
            reason:
                ScanReason::SpatialIndexUnusable {
                    property, detail, ..
                },
            ..
        } => {
            assert_eq!(property, "stops[].geo");
            assert!(
                detail.contains("wildcard"),
                "EXPLAIN must name the reason: {detail}"
            );
            // And it must name the remedy: one concrete element.
            assert!(detail.contains("stops.0.geo"), "{detail}");
        }
        other => panic!(
            "expected an annotated fallback scan, got {}",
            other.describe()
        ),
    }
}

/// A wildcard's distance is the MINIMUM over the node's matched geometries,
/// which is a well-defined total order over nodes but NOT the order any single
/// cell-ring scan emits. The `Sort` must therefore be retained.
///
/// This asserts the negative condition directly, because it is an easy line to
/// omit: everything still looks right in a small test, and it only misbehaves
/// under keyset pagination at scale, where it drops and duplicates rows.
#[test]
fn a_wildcard_path_keeps_its_explicit_sort() {
    let planner = planner_with(ready(vec![11, 8, 6]));
    let sort = LogicalPlan::Sort {
        input: Box::new(filtered(dwithin("stops[].geo", 8.54, 47.37, 500.0))),
        sort_exprs: vec![SortExpr {
            expr: func(
                "ST_DISTANCE",
                vec![geom_source("stops[].geo"), st_point(8.54, 47.37)],
                DataType::Double,
            ),
            ascending: true,
            nulls_first: false,
        }],
    };
    let plan = planner.plan(&sort).unwrap();
    assert!(
        matches!(&plan, PhysicalPlan::Sort { .. } | PhysicalPlan::TopN { .. }),
        "a wildcard path must keep an explicit Sort, got {}",
        plan.describe()
    );
}

// ── historical reads, and the pruned index ─────────────────────────────────

/// A scan scoped to an explicit revision, as the analyzer leaves it: the
/// `__revision = N` predicate is stripped into `max_revision` on the Scan node.
fn filtered_at_revision(predicate: TypedExpr, revision: raisin_hlc::HLC) -> LogicalPlan {
    let LogicalPlan::Scan {
        table,
        alias,
        schema,
        workspace,
        branch_override,
        locales,
        filter,
        projection,
        ..
    } = scan()
    else {
        unreachable!("scan() builds a Scan")
    };
    LogicalPlan::Filter {
        input: Box::new(LogicalPlan::Scan {
            table,
            alias,
            schema,
            workspace,
            max_revision: Some(revision),
            branch_override,
            locales,
            filter,
            projection,
        }),
        predicate: FilterPredicate::from_expr(predicate),
    }
}

/// HEAD is exact and MUST keep using the index.
///
/// The compaction filter never prunes the newest entry per node per cell, so a
/// read at HEAD is unaffected by pruning. This is the half of the historical gate
/// that is easy to break by making it too broad — routing HEAD queries to a row
/// scan would undo the entire spatial performance story, silently and only under
/// load.
#[test]
fn a_head_query_still_takes_the_spatial_index() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 500.0)))
        .unwrap();
    assert!(
        matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. }),
        "a HEAD spatial query must use the index, got {}",
        plan.describe()
    );
}

/// A read at an EXPLICIT older revision must not: the index is pruned beyond its
/// retention window, so it can only answer approximately there. The row scan is
/// exact at any revision.
#[test]
fn an_explicit_historical_revision_falls_back_to_a_row_scan() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    let plan = planner
        .plan(&filtered_at_revision(
            dwithin("location", 8.54, 47.37, 500.0),
            raisin_hlc::HLC::now(),
        ))
        .unwrap();

    assert!(
        !matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. }),
        "a historical spatial read must NOT resolve against the pruned index, got {}",
        plan.describe()
    );
    assert!(
        has_spatial_residual_filter(&plan),
        "the predicate must be re-applied per row, or the historical query returns \
         whatever the fallback scan happened to select: {}",
        plan.describe()
    );

    let PhysicalPlan::TableScan { reason, .. } = leaf(&plan) else {
        panic!("expected a TableScan fallback, got {}", plan.describe());
    };
    let ScanReason::SpatialIndexUnusable { detail, .. } = reason else {
        panic!("expected SpatialIndexUnusable, got {reason}");
    };
    assert!(
        detail.contains("historical revision"),
        "EXPLAIN must say WHY the index was skipped, got: {detail}"
    );
}

// ── the per-cell budget degrades, it does not fail ─────────────────────────

/// The index scan carries the plan it degrades to, and EXPLAIN says so.
///
/// A per-cell budget exhaustion is only discoverable while scanning, so the
/// executor cannot re-plan on its own — the predicate may have been stripped from
/// the residual filter. Carrying the fallback in the plan is what turns a failed
/// query into a slow one.
#[test]
fn an_index_scan_carries_the_fallback_it_degrades_to() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 500.0)))
        .unwrap();

    let PhysicalPlan::SpatialDistanceScan { fallback, .. } = leaf(&plan) else {
        panic!("expected a SpatialDistanceScan, got {}", plan.describe());
    };
    let fallback = fallback
        .as_ref()
        .expect("an index scan must carry a degradation fallback");

    // The fallback re-applies EVERY predicate per row, including the spatial one
    // the index scan was allowed to strip. Without that, degrading would return
    // every row in the workspace.
    assert!(
        has_spatial_residual_filter(fallback),
        "the fallback must re-apply the spatial predicate, got {}",
        fallback.describe()
    );
    assert!(
        leaf(&plan)
            .describe()
            .contains("degrades to a row scan if the per-cell budget is exhausted"),
        "EXPLAIN must show the degradation path: {}",
        leaf(&plan).describe()
    );
}

// ── the spatial pseudo-columns ────────────────────────────────────────────

/// Asking for `__distance` / `__matched_path` over a fallback scan plans the
/// annotation operator that materialises them. Without it the columns analyze
/// (they are declared in the catalog) and then come back NULL, which is a worse
/// failure than "column not found".
#[test]
fn selecting_the_spatial_columns_plans_the_annotation() {
    let planner = planner_with(SpatialAvailability::NotBuilt);
    let LogicalPlan::Filter { input, predicate } =
        filtered(dwithin("stops[].geo", 8.54, 47.37, 500.0))
    else {
        unreachable!()
    };
    let LogicalPlan::Scan {
        table,
        alias,
        schema,
        workspace,
        max_revision,
        branch_override,
        locales,
        filter,
        ..
    } = *input
    else {
        unreachable!()
    };
    let scoped = LogicalPlan::Filter {
        input: Box::new(LogicalPlan::Scan {
            table,
            alias,
            schema,
            workspace,
            max_revision,
            branch_override,
            locales,
            filter,
            projection: Some(vec![
                "name".to_string(),
                "properties".to_string(),
                "__distance".to_string(),
                "__matched_path".to_string(),
            ]),
        }),
        predicate,
    };

    let plan = planner.plan(&scoped).unwrap();
    assert!(
        plan.describe().contains("SpatialAnnotate")
            || matches!(&plan, PhysicalPlan::SpatialAnnotate { .. }),
        "expected the annotation operator, got {}",
        plan.describe()
    );
}

/// A projection that does NOT ask for them plans no annotation, so an ordinary
/// spatial fallback pays nothing for a feature it is not using.
#[test]
fn a_projection_without_the_spatial_columns_plans_no_annotation() {
    let planner = planner_with(SpatialAvailability::NotBuilt);
    let LogicalPlan::Filter { input, predicate } =
        filtered(dwithin("stops[].geo", 8.54, 47.37, 500.0))
    else {
        unreachable!()
    };
    let LogicalPlan::Scan {
        table,
        alias,
        schema,
        workspace,
        max_revision,
        branch_override,
        locales,
        filter,
        ..
    } = *input
    else {
        unreachable!()
    };
    let scoped = LogicalPlan::Filter {
        input: Box::new(LogicalPlan::Scan {
            table,
            alias,
            schema,
            workspace,
            max_revision,
            branch_override,
            locales,
            filter,
            projection: Some(vec!["name".to_string(), "properties".to_string()]),
        }),
        predicate,
    };

    let plan = planner.plan(&scoped).unwrap();
    assert!(
        !matches!(&plan, PhysicalPlan::SpatialAnnotate { .. }),
        "no annotation should be planned when nobody asked for the columns: {}",
        plan.describe()
    );
}

// ── ST_3DDWITHIN takes the 2-D access path, but never exactly ──────────────

/// `ST_3DDWITHIN(g, <const>, d)` with a 3-ordinate centre.
fn dwithin_3d(property: &str, lon: f64, lat: f64, z: f64, radius: f64) -> TypedExpr {
    let center = func(
        "ST_FORCE3D",
        vec![st_point(lon, lat), TypedExpr::literal(Literal::Double(z))],
        DataType::Geometry,
    );
    func(
        "ST_3DDWITHIN",
        vec![
            geom_source(property),
            center,
            TypedExpr::literal(Literal::Double(radius)),
        ],
        DataType::Boolean,
    )
}

/// A 3D proximity test must narrow through the 2-D index.
///
/// Horizontal distance is never greater than 3D distance, so the cell ring of
/// radius `d` is a conservative superset — it cannot drop a row the 3D predicate
/// would have kept. Before this, `ST_3DDWITHIN` was not recognised at all and a
/// tracking query over altitude read the whole workspace.
#[test]
fn a_3d_proximity_test_still_drives_the_spatial_scan() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    let plan = planner
        .plan(&filtered(dwithin_3d("position", 8.54, 47.37, 400.0, 500.0)))
        .unwrap();

    assert!(
        matches!(leaf(&plan), PhysicalPlan::SpatialDistanceScan { .. }),
        "ST_3DDWITHIN should narrow through the 2-D index, got {}",
        plan.describe()
    );
}

/// ...and the predicate must SURVIVE, because the index cannot answer altitude.
///
/// This is the half that makes the narrowing safe. Stripping it would return
/// every row within `d` on the ground regardless of altitude — silently wrong
/// results, which is strictly worse than the full scan it replaced.
#[test]
fn a_3d_proximity_test_is_never_stripped_from_the_plan() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    let plan = planner
        .plan(&filtered(dwithin_3d("position", 8.54, 47.37, 400.0, 500.0)))
        .unwrap();

    assert!(
        has_spatial_residual_filter(&plan),
        "ST_3DDWITHIN must be re-applied above the scan — the 2-D index cannot \
         answer the altitude component: {}",
        plan.describe()
    );
}

/// A plain 2-D `ST_DWITHIN` must still be strippable — the 3D change must not
/// pessimise the common case into always carrying a residual filter.
#[test]
fn the_2d_predicate_is_still_exact_after_the_3d_change() {
    let planner = planner_with(ready(vec![11, 10, 9, 8, 7, 6, 4, 2]));
    let plan = planner
        .plan(&filtered(dwithin("location", 8.54, 47.37, 500.0)))
        .unwrap();
    assert!(!has_spatial_residual_filter(&plan));
}
