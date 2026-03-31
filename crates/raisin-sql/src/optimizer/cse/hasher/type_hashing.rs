//! Type and value hashing helpers
//!
//! Provides stable hashing for literals, data types, operators,
//! window functions, and frame specifications.

use crate::analyzer::{
    typed_expr::{
        BinaryOperator, FrameBound, FrameMode, Literal, UnaryOperator, WindowFrame, WindowFunction,
    },
    DataType,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;

use super::expr_hashing::ExprHasher;

/// Hash a literal value
pub(super) fn hash_literal(lit: &Literal, hasher: &mut DefaultHasher) {
    match lit {
        Literal::Null => 0u8.hash(hasher),
        Literal::Boolean(b) => {
            1u8.hash(hasher);
            b.hash(hasher);
        }
        Literal::Int(i) => {
            2u8.hash(hasher);
            i.hash(hasher);
        }
        Literal::BigInt(i) => {
            3u8.hash(hasher);
            i.hash(hasher);
        }
        Literal::Double(f) => {
            4u8.hash(hasher);
            f.to_bits().hash(hasher);
        }
        Literal::Text(s) => {
            5u8.hash(hasher);
            s.hash(hasher);
        }
        Literal::Uuid(u) => {
            6u8.hash(hasher);
            u.hash(hasher);
        }
        Literal::Path(p) => {
            7u8.hash(hasher);
            p.hash(hasher);
        }
        Literal::JsonB(j) => {
            8u8.hash(hasher);
            j.to_string().hash(hasher);
        }
        Literal::Vector(v) => {
            9u8.hash(hasher);
            v.len().hash(hasher);
            for f in v {
                f.to_bits().hash(hasher);
            }
        }
        Literal::Timestamp(dt) => {
            10u8.hash(hasher);
            dt.timestamp_micros().hash(hasher);
        }
        Literal::Interval(dur) => {
            11u8.hash(hasher);
            dur.num_microseconds().unwrap_or(0).hash(hasher);
        }
        Literal::Parameter(p) => {
            12u8.hash(hasher);
            p.hash(hasher);
        }
        Literal::Geometry(g) => {
            13u8.hash(hasher);
            g.to_string().hash(hasher);
        }
    }
}

/// Hash a binary operator
pub(super) fn hash_binary_op(op: &BinaryOperator, hasher: &mut DefaultHasher) {
    std::mem::discriminant(op).hash(hasher);
}

/// Hash a unary operator
pub(super) fn hash_unary_op(op: &UnaryOperator, hasher: &mut DefaultHasher) {
    std::mem::discriminant(op).hash(hasher);
}

/// Hash a data type
pub(super) fn hash_data_type(dt: &DataType, hasher: &mut DefaultHasher) {
    match dt {
        DataType::Unknown => 0u8.hash(hasher),
        DataType::Boolean => 1u8.hash(hasher),
        DataType::Int => 2u8.hash(hasher),
        DataType::BigInt => 3u8.hash(hasher),
        DataType::Double => 4u8.hash(hasher),
        DataType::Text => 5u8.hash(hasher),
        DataType::Uuid => 6u8.hash(hasher),
        DataType::Path => 7u8.hash(hasher),
        DataType::JsonB => 8u8.hash(hasher),
        DataType::TimestampTz => 9u8.hash(hasher),
        DataType::TSVector => 10u8.hash(hasher),
        DataType::TSQuery => 11u8.hash(hasher),
        DataType::Vector(dim) => {
            12u8.hash(hasher);
            dim.hash(hasher);
        }
        DataType::Nullable(inner) => {
            13u8.hash(hasher);
            hash_data_type(inner, hasher);
        }
        DataType::Array(elem_type) => {
            14u8.hash(hasher);
            hash_data_type(elem_type, hasher);
        }
        DataType::Interval => 15u8.hash(hasher),
        DataType::Geometry => 16u8.hash(hasher),
    }
}

/// Hash a window function
pub(super) fn hash_window_function(func: &WindowFunction, hasher: &mut DefaultHasher) {
    match func {
        WindowFunction::RowNumber => 0u8.hash(hasher),
        WindowFunction::Rank => 1u8.hash(hasher),
        WindowFunction::DenseRank => 2u8.hash(hasher),
        WindowFunction::Count => 3u8.hash(hasher),
        WindowFunction::Sum(expr) => {
            4u8.hash(hasher);
            ExprHasher::hash_typed_expr(expr, hasher);
        }
        WindowFunction::Avg(expr) => {
            5u8.hash(hasher);
            ExprHasher::hash_typed_expr(expr, hasher);
        }
        WindowFunction::Min(expr) => {
            6u8.hash(hasher);
            ExprHasher::hash_typed_expr(expr, hasher);
        }
        WindowFunction::Max(expr) => {
            7u8.hash(hasher);
            ExprHasher::hash_typed_expr(expr, hasher);
        }
    }
}

/// Hash a window frame specification
pub(super) fn hash_window_frame(frame: &WindowFrame, hasher: &mut DefaultHasher) {
    match frame.mode {
        FrameMode::Rows => 0u8.hash(hasher),
        FrameMode::Range => 1u8.hash(hasher),
    }

    hash_frame_bound(&frame.start, hasher);

    if let Some(end) = &frame.end {
        true.hash(hasher);
        hash_frame_bound(end, hasher);
    } else {
        false.hash(hasher);
    }
}

/// Hash a frame bound
fn hash_frame_bound(bound: &FrameBound, hasher: &mut DefaultHasher) {
    match bound {
        FrameBound::UnboundedPreceding => 0u8.hash(hasher),
        FrameBound::Preceding(n) => {
            1u8.hash(hasher);
            n.hash(hasher);
        }
        FrameBound::CurrentRow => 2u8.hash(hasher),
        FrameBound::Following(n) => {
            3u8.hash(hasher);
            n.hash(hasher);
        }
        FrameBound::UnboundedFollowing => 4u8.hash(hasher),
    }
}
