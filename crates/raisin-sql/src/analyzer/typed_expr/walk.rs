//! Child traversal for [`TypedExpr`].
//!
//! ONE exhaustive enumeration of "which sub-expressions does this node own",
//! shared by every pass that needs to walk or rewrite an expression tree
//! (subquery binding, HAVING rewriting, column-reference collection). A new
//! `Expr` variant fails to compile here until it says what its children are,
//! which is the point: a pass with a `_ => {}` arm silently skips a variant it
//! has never heard of.

use super::expressions::{Expr, TypedExpr};

impl TypedExpr {
    /// Visit every direct child expression. Subquery bodies are NOT children:
    /// they are a separate scope with their own binding pass.
    pub fn for_each_child<'a>(&'a self, f: &mut dyn FnMut(&'a TypedExpr)) {
        match &self.expr {
            Expr::Literal(_) | Expr::Column { .. } => {}
            Expr::Function { args, filter, .. } => {
                args.iter().for_each(|a| f(a));
                if let Some(x) = filter {
                    f(x);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                f(left);
                f(right);
            }
            Expr::UnaryOp { expr, .. }
            | Expr::Cast { expr, .. }
            | Expr::IsNull { expr }
            | Expr::IsNotNull { expr } => f(expr),
            Expr::Between { expr, low, high } => {
                f(expr);
                f(low);
                f(high);
            }
            Expr::InList { expr, list, .. } => {
                f(expr);
                list.iter().for_each(|x| f(x));
            }
            Expr::InSubquery { expr, .. } => f(expr),
            Expr::Exists { .. } | Expr::ScalarSubquery { .. } => {}
            Expr::Quantified { left, right, .. } => {
                f(left);
                f(right);
            }
            Expr::QuantifiedSubquery { left, .. } => f(left),
            Expr::Regex { expr, pattern, .. }
            | Expr::Like { expr, pattern, .. }
            | Expr::ILike { expr, pattern, .. } => {
                f(expr);
                f(pattern);
            }
            Expr::JsonExtract { object, key }
            | Expr::JsonExtractText { object, key }
            | Expr::JsonContains {
                object,
                pattern: key,
            }
            | Expr::JsonKeyExists { object, key }
            | Expr::JsonAnyKeyExists { object, keys: key }
            | Expr::JsonAllKeyExists { object, keys: key }
            | Expr::JsonExtractPath { object, path: key }
            | Expr::JsonExtractPathText { object, path: key }
            | Expr::JsonRemove { object, key }
            | Expr::JsonRemoveAtPath { object, path: key }
            | Expr::JsonPathMatch { object, path: key }
            | Expr::JsonPathExists { object, path: key } => {
                f(object);
                f(key);
            }
            Expr::Window {
                function,
                partition_by,
                order_by,
                ..
            } => {
                use super::window::WindowFunction;
                match function {
                    WindowFunction::Sum(e)
                    | WindowFunction::Avg(e)
                    | WindowFunction::Min(e)
                    | WindowFunction::Max(e) => f(e),
                    WindowFunction::RowNumber
                    | WindowFunction::Rank
                    | WindowFunction::DenseRank
                    | WindowFunction::Count => {}
                }
                partition_by.iter().for_each(|x| f(x));
                order_by.iter().for_each(|(x, _)| f(x));
            }
            Expr::Case {
                conditions,
                else_expr,
            } => {
                for (c, r) in conditions {
                    f(c);
                    f(r);
                }
                if let Some(e) = else_expr {
                    f(e);
                }
            }
        }
    }

    /// Mutable twin of [`Self::for_each_child`].
    pub fn for_each_child_mut(&mut self, f: &mut dyn FnMut(&mut TypedExpr)) {
        match &mut self.expr {
            Expr::Literal(_) | Expr::Column { .. } => {}
            Expr::Function { args, filter, .. } => {
                args.iter_mut().for_each(|a| f(a));
                if let Some(x) = filter {
                    f(x);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                f(left);
                f(right);
            }
            Expr::UnaryOp { expr, .. }
            | Expr::Cast { expr, .. }
            | Expr::IsNull { expr }
            | Expr::IsNotNull { expr } => f(expr),
            Expr::Between { expr, low, high } => {
                f(expr);
                f(low);
                f(high);
            }
            Expr::InList { expr, list, .. } => {
                f(expr);
                list.iter_mut().for_each(|x| f(x));
            }
            Expr::InSubquery { expr, .. } => f(expr),
            Expr::Exists { .. } | Expr::ScalarSubquery { .. } => {}
            Expr::Quantified { left, right, .. } => {
                f(left);
                f(right);
            }
            Expr::QuantifiedSubquery { left, .. } => f(left),
            Expr::Regex { expr, pattern, .. }
            | Expr::Like { expr, pattern, .. }
            | Expr::ILike { expr, pattern, .. } => {
                f(expr);
                f(pattern);
            }
            Expr::JsonExtract { object, key }
            | Expr::JsonExtractText { object, key }
            | Expr::JsonContains {
                object,
                pattern: key,
            }
            | Expr::JsonKeyExists { object, key }
            | Expr::JsonAnyKeyExists { object, keys: key }
            | Expr::JsonAllKeyExists { object, keys: key }
            | Expr::JsonExtractPath { object, path: key }
            | Expr::JsonExtractPathText { object, path: key }
            | Expr::JsonRemove { object, key }
            | Expr::JsonRemoveAtPath { object, path: key }
            | Expr::JsonPathMatch { object, path: key }
            | Expr::JsonPathExists { object, path: key } => {
                f(object);
                f(key);
            }
            Expr::Window {
                function,
                partition_by,
                order_by,
                ..
            } => {
                use super::window::WindowFunction;
                match function {
                    WindowFunction::Sum(e)
                    | WindowFunction::Avg(e)
                    | WindowFunction::Min(e)
                    | WindowFunction::Max(e) => f(e),
                    WindowFunction::RowNumber
                    | WindowFunction::Rank
                    | WindowFunction::DenseRank
                    | WindowFunction::Count => {}
                }
                partition_by.iter_mut().for_each(|x| f(x));
                order_by.iter_mut().for_each(|(x, _)| f(x));
            }
            Expr::Case {
                conditions,
                else_expr,
            } => {
                for (c, r) in conditions {
                    f(c);
                    f(r);
                }
                if let Some(e) = else_expr {
                    f(e);
                }
            }
        }
    }

    /// Pre-order walk over this expression and all descendants.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a TypedExpr)) {
        f(self);
        self.for_each_child(&mut |c| c.walk(f));
    }

    /// True if any node in the tree satisfies `pred`.
    pub fn any(&self, pred: &dyn Fn(&TypedExpr) -> bool) -> bool {
        let mut hit = false;
        self.walk(&mut |e| {
            if !hit && pred(e) {
                hit = true;
            }
        });
        hit
    }

    /// Rewrite bottom-up: children first, then `f` on the node itself.
    pub fn rewrite(&mut self, f: &mut dyn FnMut(&mut TypedExpr)) {
        self.for_each_child_mut(&mut |c| c.rewrite(f));
        f(self);
    }

    /// True if the tree holds a subquery the engine still has to bind
    /// (EXISTS, scalar, quantified-over-subquery). `IN (subquery)` is not
    /// included — it is planned as a semi-join, not bound to a literal.
    pub fn has_unbound_subquery(&self) -> bool {
        self.any(&|e| {
            matches!(
                e.expr,
                Expr::Exists { .. } | Expr::ScalarSubquery { .. } | Expr::QuantifiedSubquery { .. }
            )
        })
    }
}
