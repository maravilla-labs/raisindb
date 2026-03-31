//! Supporting types for logical plan operators

use crate::analyzer::{typed_expr::BinaryOperator, ColumnDef, DataType, Expr, TypedExpr};

/// Specification for DISTINCT behavior
#[derive(Debug, Clone)]
pub enum DistinctSpec {
    /// DISTINCT: deduplicate based on all output columns
    All,
    /// DISTINCT ON (expressions): deduplicate based on specific expressions
    /// PostgreSQL semantics: keep first row per distinct-on value according to ORDER BY
    On(Vec<TypedExpr>),
}

/// Window expression definition
#[derive(Debug, Clone)]
pub struct WindowExpr {
    pub function: crate::analyzer::WindowFunction,
    pub partition_by: Vec<TypedExpr>,
    pub order_by: Vec<(TypedExpr, bool)>, // (expr, is_desc)
    pub frame: Option<crate::analyzer::WindowFrame>,
    pub alias: String,
    pub return_type: DataType,
}

/// Filter predicate in Conjunctive Normal Form (CNF)
/// Represents multiple predicates that are AND-ed together
#[derive(Debug, Clone)]
pub struct FilterPredicate {
    /// Individual AND-ed predicates (conjuncts)
    pub conjuncts: Vec<TypedExpr>,
    /// Canonical predicates after hierarchy rewriting (set by optimizer)
    /// This allows the physical planner to efficiently select index scans
    pub canonical: Option<Vec<crate::optimizer::hierarchy_rewrite::CanonicalPredicate>>,
}

impl FilterPredicate {
    /// Create from a single predicate, flattening ANDs
    pub fn from_expr(expr: TypedExpr) -> Self {
        let conjuncts = Self::flatten_ands(expr);
        Self {
            conjuncts,
            canonical: None, // Will be set by optimizer
        }
    }

    /// Flatten nested AND operations into a list of conjuncts
    fn flatten_ands(expr: TypedExpr) -> Vec<TypedExpr> {
        match expr.expr {
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } => {
                let mut result = Self::flatten_ands(*left);
                result.extend(Self::flatten_ands(*right));
                result
            }
            _ => vec![expr],
        }
    }

    /// Convert back to a single expression by AND-ing all conjuncts
    pub fn to_expr(&self) -> Option<TypedExpr> {
        if self.conjuncts.is_empty() {
            return None;
        }

        if self.conjuncts.len() == 1 {
            return Some(self.conjuncts[0].clone());
        }

        // Build nested AND tree
        let mut result = self.conjuncts[0].clone();
        for conjunct in &self.conjuncts[1..] {
            result = TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(result),
                    op: BinaryOperator::And,
                    right: Box::new(conjunct.clone()),
                },
                DataType::Boolean,
            );
        }

        Some(result)
    }
}

/// Projection expression with alias
#[derive(Debug, Clone)]
pub struct ProjectionExpr {
    pub expr: TypedExpr,
    pub alias: String,
}

/// Sort expression with direction and nulls ordering
#[derive(Debug, Clone)]
pub struct SortExpr {
    pub expr: TypedExpr,
    pub ascending: bool,
    /// Nulls ordering: true = NULLS FIRST, false = NULLS LAST
    pub nulls_first: bool,
}

/// Aggregate expression
#[derive(Debug, Clone)]
pub struct AggregateExpr {
    pub func: AggregateFunction,
    pub args: Vec<TypedExpr>,
    pub alias: String,
    pub return_type: DataType,
    /// Optional FILTER clause: e.g., COUNT(*) FILTER (WHERE x > 10)
    pub filter: Option<TypedExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AggregateFunction {
    Count,
    CountDistinct,
    Sum,
    Avg,
    Min,
    Max,
    ArrayAgg,
}

/// Table schema for scan operations
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub table_name: String,
    pub columns: Vec<ColumnDef>,
}

/// Schema column (simplified version for schema output)
#[derive(Debug, Clone)]
pub struct SchemaColumn {
    pub name: String,
    pub data_type: DataType,
}
