//! Binary and unary operator types with type inference
//!
//! Defines the operators used in typed expressions and their result type
//! computation based on operand types.

use crate::analyzer::types::DataType;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOperator {
    // Arithmetic
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,

    // Comparison
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,

    // Logical
    And,
    Or,

    // JSON operators
    JsonExtract,  // ->>
    JsonContains, // @>
    JsonConcat,   // || (JSONB concatenation/merge)

    // String concatenation
    StringConcat, // || (text concatenation)

    // Full-text search operator
    TextSearchMatch, // @@ (PostgreSQL text search match)

    // Vector distance operators (pgvector-compatible)
    VectorL2Distance,     // <-> Euclidean distance (L2)
    VectorCosineDistance, // <=> Cosine distance (for normalized vectors)
    VectorInnerProduct,   // <#> Inner product (negative dot product)
}

impl BinaryOperator {
    /// Get the return type for this operator given operand types
    pub fn result_type(&self, left: &DataType, right: &DataType) -> Option<DataType> {
        match self {
            // Arithmetic operators
            BinaryOperator::Add | BinaryOperator::Subtract => {
                // Special handling for timestamp arithmetic
                match (left.base_type(), right.base_type()) {
                    // TIMESTAMPTZ +/- INTERVAL -> TIMESTAMPTZ
                    (DataType::TimestampTz, DataType::Interval) => Some(DataType::TimestampTz),
                    // INTERVAL + TIMESTAMPTZ -> TIMESTAMPTZ (commutative for Add)
                    (DataType::Interval, DataType::TimestampTz)
                        if matches!(self, BinaryOperator::Add) =>
                    {
                        Some(DataType::TimestampTz)
                    }
                    // TIMESTAMPTZ - TIMESTAMPTZ -> INTERVAL
                    (DataType::TimestampTz, DataType::TimestampTz)
                        if matches!(self, BinaryOperator::Subtract) =>
                    {
                        Some(DataType::Interval)
                    }
                    // Otherwise use common numeric type
                    _ => left.common_type(right),
                }
            }
            BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Modulo => {
                left.common_type(right)
            }

            // Comparison operators return boolean
            BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq => {
                // Check if types are comparable
                if left.common_type(right).is_some()
                    || matches!(
                        (left.base_type(), right.base_type()),
                        (DataType::TimestampTz, DataType::TimestampTz)
                            // Allow TimestampTz vs Text comparison for cursor-based pagination
                            // The text will be parsed as a timestamp at evaluation time
                            | (DataType::TimestampTz, DataType::Text)
                            | (DataType::Text, DataType::TimestampTz)
                    )
                {
                    Some(DataType::Boolean)
                } else {
                    None
                }
            }

            // Logical operators require boolean operands
            BinaryOperator::And | BinaryOperator::Or => {
                if matches!(left.base_type(), DataType::Boolean)
                    && matches!(right.base_type(), DataType::Boolean)
                {
                    Some(DataType::Boolean)
                } else {
                    None
                }
            }

            // JSON extract returns nullable text
            BinaryOperator::JsonExtract => {
                if matches!(left.base_type(), DataType::JsonB)
                    && matches!(right.base_type(), DataType::Text)
                {
                    Some(DataType::Nullable(Box::new(DataType::Text)))
                } else {
                    None
                }
            }

            // JSON contains returns boolean
            BinaryOperator::JsonContains => {
                if matches!(left.base_type(), DataType::JsonB) {
                    Some(DataType::Boolean)
                } else {
                    None
                }
            }

            // JSON concatenation/merge: JSONB || JSONB -> JSONB
            // Merges the right object into the left object (PostgreSQL semantics)
            BinaryOperator::JsonConcat => {
                if matches!(left.base_type(), DataType::JsonB)
                    && matches!(right.base_type(), DataType::JsonB)
                {
                    Some(DataType::JsonB)
                } else {
                    None
                }
            }

            // String concatenation: Text || Text -> Text
            // Also works with Path types (coerced to Text)
            BinaryOperator::StringConcat => Some(DataType::Text),

            // Full-text search match returns boolean
            BinaryOperator::TextSearchMatch => {
                if matches!(left.base_type(), DataType::TSVector)
                    && matches!(right.base_type(), DataType::TSQuery)
                {
                    Some(DataType::Boolean)
                } else {
                    None
                }
            }

            // Vector distance operators return DOUBLE
            // Both operands must be Vector types with matching dimensions
            BinaryOperator::VectorL2Distance
            | BinaryOperator::VectorCosineDistance
            | BinaryOperator::VectorInnerProduct => {
                match (left.base_type(), right.base_type()) {
                    (DataType::Vector(left_dim), DataType::Vector(right_dim)) => {
                        // Dimensions must match
                        if left_dim == right_dim {
                            Some(DataType::Double)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOperator {
    Not,
    Negate,
}

impl UnaryOperator {
    /// Get the return type for this operator given operand type
    pub fn result_type(&self, operand: &DataType) -> Option<DataType> {
        match self {
            UnaryOperator::Not => {
                if matches!(operand.base_type(), DataType::Boolean) {
                    Some(DataType::Boolean)
                } else {
                    None
                }
            }
            UnaryOperator::Negate => match operand.base_type() {
                DataType::Int => Some(DataType::Int),
                DataType::BigInt => Some(DataType::BigInt),
                DataType::Double => Some(DataType::Double),
                _ => None,
            },
        }
    }
}
