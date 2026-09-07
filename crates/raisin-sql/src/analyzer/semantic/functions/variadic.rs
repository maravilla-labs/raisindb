//! Variadic scalar functions: CONCAT, CONCAT_WS, GREATEST, LEAST, FORMAT.
//!
//! The registry resolves by exact arity, so these are typed here instead:
//! the return type is fixed (TEXT) or the common type of the arguments
//! (GREATEST / LEAST), and the signature is synthesised from the actual
//! argument list.

use super::super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    functions::{FunctionCategory, FunctionSignature},
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};

impl<'a> AnalyzerContext<'a> {
    /// Type a variadic scalar call, or `None` when `name` is not one.
    pub(in crate::analyzer::semantic) fn analyze_variadic_scalar(
        &self,
        name: &str,
        args: &[TypedExpr],
    ) -> Result<Option<TypedExpr>> {
        let (min_args, return_type) = match name {
            "CONCAT" | "FORMAT" => (1, DataType::Text),
            "CONCAT_WS" => (2, DataType::Text),
            "GREATEST" | "LEAST" => (1, self.common_arg_type(name, args)?),
            _ => return Ok(None),
        };
        if args.len() < min_args {
            return Err(AnalysisError::FunctionNotFound {
                name: name.to_string(),
                args: format!("{} requires at least {} argument(s)", name, min_args),
            });
        }

        let signature = FunctionSignature {
            name: name.to_string(),
            params: args.iter().map(|a| a.data_type.clone()).collect(),
            return_type: return_type.clone(),
            is_deterministic: true,
            category: FunctionCategory::Scalar,
        };

        if args.iter().all(|a| matches!(a.expr, Expr::Literal(_))) {
            if let Some(folded) = self.try_constant_fold(name, args)? {
                return Ok(Some(folded));
            }
        }

        Ok(Some(TypedExpr::new(
            Expr::Function {
                name: name.to_string(),
                args: args.to_vec(),
                signature,
                filter: None,
            },
            return_type,
        )))
    }

    /// The common type of all arguments (numeric ladder, TEXT/PATH), nullable
    /// because every argument may be NULL.
    fn common_arg_type(&self, name: &str, args: &[TypedExpr]) -> Result<DataType> {
        let mut common = DataType::Unknown;
        for arg in args {
            let arg_base = arg.data_type.base_type();
            common = match common.common_type(arg_base) {
                Some(t) => t.base_type().clone(),
                None => {
                    return Err(AnalysisError::TypeMismatch {
                        expected: format!("{}: {}", name, common),
                        actual: arg.data_type.to_string(),
                    })
                }
            };
        }
        Ok(common.as_nullable())
    }
}
