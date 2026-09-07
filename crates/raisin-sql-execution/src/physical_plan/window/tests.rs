use super::aggregates::extract_number;
use super::compare::compare_literals;
use super::frame::determine_frame_bounds;
use raisin_sql::analyzer::{FrameBound, FrameMode, Literal};
use std::cmp::Ordering;

use super::PartitionKey;

#[test]
fn test_partition_key_equality() {
    let key1 = PartitionKey {
        values: vec!["a".to_string(), "b".to_string()],
    };
    let key2 = PartitionKey {
        values: vec!["a".to_string(), "b".to_string()],
    };
    let key3 = PartitionKey {
        values: vec!["a".to_string(), "c".to_string()],
    };

    assert_eq!(key1, key2);
    assert_ne!(key1, key3);
}

#[test]
fn test_compare_literals_numeric() {
    assert_eq!(
        compare_literals(&Literal::Int(5), &Literal::Int(10)),
        Ordering::Less
    );
    assert_eq!(
        compare_literals(&Literal::Int(10), &Literal::Int(5)),
        Ordering::Greater
    );
    assert_eq!(
        compare_literals(&Literal::Int(5), &Literal::Int(5)),
        Ordering::Equal
    );
}

#[test]
fn test_compare_literals_null() {
    assert_eq!(
        compare_literals(&Literal::Null, &Literal::Int(5)),
        Ordering::Less
    );
    assert_eq!(
        compare_literals(&Literal::Int(5), &Literal::Null),
        Ordering::Greater
    );
    assert_eq!(
        compare_literals(&Literal::Null, &Literal::Null),
        Ordering::Equal
    );
}

#[test]
fn test_compare_literals_text() {
    assert_eq!(
        compare_literals(&Literal::Text("abc".into()), &Literal::Text("def".into())),
        Ordering::Less
    );
    assert_eq!(
        compare_literals(&Literal::Text("xyz".into()), &Literal::Text("abc".into())),
        Ordering::Greater
    );
}

#[test]
fn test_frame_bounds_unbounded() {
    // UNBOUNDED PRECEDING to UNBOUNDED FOLLOWING = entire partition
    let frame = raisin_sql::analyzer::WindowFrame {
        mode: FrameMode::Rows,
        start: FrameBound::UnboundedPreceding,
        end: Some(FrameBound::UnboundedFollowing),
    };

    let (start, end) = determine_frame_bounds(5, 10, &Some(frame), true);
    assert_eq!(start, 0);
    assert_eq!(end, 10);
}

#[test]
fn test_frame_bounds_current_row() {
    // CURRENT ROW to CURRENT ROW = just this row
    let frame = raisin_sql::analyzer::WindowFrame {
        mode: FrameMode::Rows,
        start: FrameBound::CurrentRow,
        end: Some(FrameBound::CurrentRow),
    };

    let (start, end) = determine_frame_bounds(5, 10, &Some(frame), true);
    assert_eq!(start, 5);
    assert_eq!(end, 6); // Exclusive end
}

#[test]
fn test_frame_bounds_preceding() {
    // 2 PRECEDING to CURRENT ROW
    let frame = raisin_sql::analyzer::WindowFrame {
        mode: FrameMode::Rows,
        start: FrameBound::Preceding(2),
        end: Some(FrameBound::CurrentRow),
    };

    let (start, end) = determine_frame_bounds(5, 10, &Some(frame), true);
    assert_eq!(start, 3); // 5 - 2
    assert_eq!(end, 6); // 5 + 1 (exclusive)
}

#[test]
fn test_frame_bounds_following() {
    // CURRENT ROW to 2 FOLLOWING
    let frame = raisin_sql::analyzer::WindowFrame {
        mode: FrameMode::Rows,
        start: FrameBound::CurrentRow,
        end: Some(FrameBound::Following(2)),
    };

    let (start, end) = determine_frame_bounds(5, 10, &Some(frame), true);
    assert_eq!(start, 5);
    assert_eq!(end, 8); // 5 + 2 + 1 (exclusive)
}

#[test]
fn test_extract_number() {
    assert_eq!(extract_number(&Literal::Int(42)), Some(42.0));
    assert_eq!(extract_number(&Literal::BigInt(100)), Some(100.0));
    assert_eq!(extract_number(&Literal::Double(3.25)), Some(3.25));
    assert_eq!(extract_number(&Literal::Text("not a number".into())), None);
}

#[test]
fn default_frame_is_running_with_order_by_and_whole_partition_without() {
    // ORDER BY present, no explicit frame: UNBOUNDED PRECEDING .. CURRENT ROW
    assert_eq!(determine_frame_bounds(0, 4, &None, true), (0, 1));
    assert_eq!(determine_frame_bounds(2, 4, &None, true), (0, 3));
    // No ORDER BY: the whole partition
    assert_eq!(determine_frame_bounds(2, 4, &None, false), (0, 4));
}

#[test]
fn rank_and_dense_rank_over_a_sorted_partition() {
    use super::ranking::RankState;
    use crate::physical_plan::executor::Row;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql::analyzer::{DataType, TypedExpr, WindowFunction};
    use raisin_sql::logical_plan::WindowExpr;

    let rows: Vec<Row> = [1, 1, 2, 3, 3, 3]
        .iter()
        .map(|v| {
            let mut r = Row::new();
            r.insert("v".to_string(), PropertyValue::Integer(*v));
            r
        })
        .collect();
    let we = WindowExpr {
        function: WindowFunction::Rank,
        partition_by: vec![],
        order_by: vec![(
            TypedExpr::column("".into(), "v".into(), DataType::Int),
            false,
        )],
        frame: None,
        alias: "r".into(),
        return_type: DataType::BigInt,
    };
    let mut state = RankState::new();
    let ranks: Vec<Literal> = (0..rows.len())
        .map(|i| state.compute_rank(i, &rows, &we))
        .collect();
    assert_eq!(
        ranks,
        vec![1, 1, 3, 4, 4, 4]
            .into_iter()
            .map(Literal::BigInt)
            .collect::<Vec<_>>()
    );
    let mut state = RankState::new();
    let dense: Vec<Literal> = (0..rows.len())
        .map(|i| state.compute_dense_rank(i, &rows, &we))
        .collect();
    assert_eq!(
        dense,
        vec![1, 1, 2, 3, 3, 3]
            .into_iter()
            .map(Literal::BigInt)
            .collect::<Vec<_>>()
    );
}
