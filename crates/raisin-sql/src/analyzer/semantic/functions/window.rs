//! Window function analysis
//!
//! Handles analysis of window functions (ROW_NUMBER, RANK, DENSE_RANK,
//! COUNT, SUM, AVG, MIN, MAX) with OVER clauses including PARTITION BY,
//! ORDER BY, and frame specifications.

use super::super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    typed_expr::{Expr, FrameBound, FrameMode, Literal, TypedExpr, WindowFrame, WindowFunction},
    types::DataType,
};

impl<'a> AnalyzerContext<'a> {
    /// Analyze a window function (function with OVER clause)
    pub(in crate::analyzer::semantic) fn analyze_window_function(
        &self,
        func_name: &str,
        args: Vec<TypedExpr>,
        over: &sqlparser::ast::WindowType,
    ) -> Result<TypedExpr> {
        use sqlparser::ast::{WindowFrameBound as SqlFrameBound, WindowFrameUnits, WindowType};

        // Extract WindowSpec from WindowType
        let spec = match over {
            WindowType::WindowSpec(spec) => spec,
            WindowType::NamedWindow(_) => {
                return Err(AnalysisError::UnsupportedExpression(
                    "Named windows (WINDOW clause) are not yet supported".into(),
                ));
            }
        };

        // Analyze PARTITION BY expressions
        let partition_by: Result<Vec<TypedExpr>> = spec
            .partition_by
            .iter()
            .map(|e| self.analyze_expr(e))
            .collect();
        let partition_by = partition_by?;

        // Analyze ORDER BY expressions
        let order_by: Result<Vec<(TypedExpr, bool)>> = spec
            .order_by
            .iter()
            .map(|ob| {
                let expr = self.analyze_expr(&ob.expr)?;
                let is_desc = ob.options.asc == Some(false);
                Ok((expr, is_desc))
            })
            .collect();
        let order_by = order_by?;

        // Analyze window frame if present
        let frame = if let Some(window_frame) = &spec.window_frame {
            let mode = match window_frame.units {
                WindowFrameUnits::Rows => FrameMode::Rows,
                WindowFrameUnits::Range => FrameMode::Range,
                WindowFrameUnits::Groups => {
                    return Err(AnalysisError::UnsupportedExpression(
                        "GROUPS frame mode is not yet supported".into(),
                    ));
                }
            };

            let convert_bound = |bound: &SqlFrameBound| -> Result<FrameBound> {
                match bound {
                    SqlFrameBound::CurrentRow => Ok(FrameBound::CurrentRow),
                    SqlFrameBound::Preceding(None) => Ok(FrameBound::UnboundedPreceding),
                    SqlFrameBound::Preceding(Some(expr)) => {
                        let typed_expr = self.analyze_expr(expr)?;
                        if let Expr::Literal(Literal::Int(n)) = typed_expr.expr {
                            Ok(FrameBound::Preceding(n as usize))
                        } else {
                            Err(AnalysisError::UnsupportedExpression(
                                "Frame bound must be a constant integer".into(),
                            ))
                        }
                    }
                    SqlFrameBound::Following(None) => Ok(FrameBound::UnboundedFollowing),
                    SqlFrameBound::Following(Some(expr)) => {
                        let typed_expr = self.analyze_expr(expr)?;
                        if let Expr::Literal(Literal::Int(n)) = typed_expr.expr {
                            Ok(FrameBound::Following(n as usize))
                        } else {
                            Err(AnalysisError::UnsupportedExpression(
                                "Frame bound must be a constant integer".into(),
                            ))
                        }
                    }
                }
            };

            let start = convert_bound(&window_frame.start_bound)?;
            let end = window_frame
                .end_bound
                .as_ref()
                .map(convert_bound)
                .transpose()?;

            let frame = WindowFrame { mode, start, end };
            frame
                .validate()
                .map_err(AnalysisError::UnsupportedExpression)?;

            Some(frame)
        } else {
            None
        };

        // Determine window function type and return type
        let (window_func, return_type) = match func_name {
            "ROW_NUMBER" => {
                if !args.is_empty() {
                    return Err(AnalysisError::UnsupportedExpression(
                        "ROW_NUMBER() does not take arguments".into(),
                    ));
                }
                (WindowFunction::RowNumber, DataType::BigInt)
            }
            "RANK" => {
                if !args.is_empty() {
                    return Err(AnalysisError::UnsupportedExpression(
                        "RANK() does not take arguments".into(),
                    ));
                }
                (WindowFunction::Rank, DataType::BigInt)
            }
            "DENSE_RANK" => {
                if !args.is_empty() {
                    return Err(AnalysisError::UnsupportedExpression(
                        "DENSE_RANK() does not take arguments".into(),
                    ));
                }
                (WindowFunction::DenseRank, DataType::BigInt)
            }
            "COUNT" => (WindowFunction::Count, DataType::BigInt),
            "SUM" => {
                if args.len() != 1 {
                    return Err(AnalysisError::UnsupportedExpression(
                        "SUM() requires exactly one argument".into(),
                    ));
                }
                let arg = args[0].clone();
                let return_type = match arg.data_type.base_type() {
                    DataType::Int | DataType::BigInt => DataType::BigInt,
                    DataType::Double => DataType::Double,
                    _ => {
                        return Err(AnalysisError::UnsupportedExpression(format!(
                            "SUM() requires numeric argument, got {}",
                            arg.data_type
                        )));
                    }
                };
                (WindowFunction::Sum(Box::new(arg)), return_type)
            }
            "AVG" => {
                if args.len() != 1 {
                    return Err(AnalysisError::UnsupportedExpression(
                        "AVG() requires exactly one argument".into(),
                    ));
                }
                let arg = args[0].clone();
                if !matches!(
                    arg.data_type.base_type(),
                    DataType::Int | DataType::BigInt | DataType::Double
                ) {
                    return Err(AnalysisError::UnsupportedExpression(format!(
                        "AVG() requires numeric argument, got {}",
                        arg.data_type
                    )));
                }
                (WindowFunction::Avg(Box::new(arg)), DataType::Double)
            }
            "MIN" => {
                if args.len() != 1 {
                    return Err(AnalysisError::UnsupportedExpression(
                        "MIN() requires exactly one argument".into(),
                    ));
                }
                let arg = args[0].clone();
                let return_type = arg.data_type.clone();
                (WindowFunction::Min(Box::new(arg)), return_type)
            }
            "MAX" => {
                if args.len() != 1 {
                    return Err(AnalysisError::UnsupportedExpression(
                        "MAX() requires exactly one argument".into(),
                    ));
                }
                let arg = args[0].clone();
                let return_type = arg.data_type.clone();
                (WindowFunction::Max(Box::new(arg)), return_type)
            }
            _ => {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "Window function {} is not supported",
                    func_name
                )));
            }
        };

        Ok(TypedExpr::new(
            Expr::Window {
                function: window_func,
                partition_by,
                order_by,
                frame,
            },
            return_type,
        ))
    }
}
