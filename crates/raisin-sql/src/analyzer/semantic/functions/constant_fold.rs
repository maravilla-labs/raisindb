//! Constant folding for deterministic functions
//!
//! Evaluates deterministic functions at analysis time when all arguments
//! are literal values (e.g., DEPTH('/a/b/c') -> 3).

use super::super::{AnalyzerContext, Result};
use crate::analyzer::typed_expr::{Expr, Literal, TypedExpr};

impl<'a> AnalyzerContext<'a> {
    /// Try to constant fold a function call
    pub(in crate::analyzer::semantic) fn try_constant_fold(
        &self,
        func_name: &str,
        args: &[TypedExpr],
    ) -> Result<Option<TypedExpr>> {
        match func_name {
            "DEPTH" => {
                if let Some(first) = args.first() {
                    let path_str = match &first.expr {
                        Expr::Literal(Literal::Path(path)) => Some(path),
                        Expr::Literal(Literal::Text(path)) => Some(path),
                        _ => None,
                    };

                    if let Some(path) = path_str {
                        let depth = path.split('/').filter(|s| !s.is_empty()).count() as i32;
                        return Ok(Some(TypedExpr::literal(Literal::Int(depth))));
                    }
                }
            }
            "PARENT" => {
                if let Some(first) = args.first() {
                    let path_str = match &first.expr {
                        Expr::Literal(Literal::Path(path)) => Some(path),
                        Expr::Literal(Literal::Text(path)) => Some(path),
                        _ => None,
                    };

                    if let Some(path) = path_str {
                        let parent = path.rsplit_once('/').map(|(p, _)| {
                            if p.is_empty() {
                                "/".to_string()
                            } else {
                                p.to_string()
                            }
                        });

                        if let Some(parent_path) = parent {
                            return Ok(Some(TypedExpr::literal(
                                if matches!(first.expr, Expr::Literal(Literal::Path(_))) {
                                    Literal::Path(parent_path)
                                } else {
                                    Literal::Text(parent_path)
                                },
                            )));
                        } else {
                            return Ok(Some(TypedExpr::literal(Literal::Null)));
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(None)
    }
}
