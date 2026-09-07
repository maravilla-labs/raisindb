//! Resolution of a scalar call: signature lookup, argument coercion and
//! analysis-time constant folding.
//!
//! Used by `analyze_function` for ordinary `NAME(args)` calls AND by the
//! special AST forms (`SUBSTRING(x FROM 1 FOR 2)`, `TRIM(BOTH 'x' FROM y)`,
//! `EXTRACT(YEAR FROM ts)`, `POSITION(a IN b)`, `CEIL`/`FLOOR`, `CAST(x AS
//! DATE)`), which the analyzer lowers onto the same named functions so there
//! is one type-check and one runtime implementation per function.

use super::super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    functions::FunctionSignature,
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};

/// Outcome of resolving a scalar call.
pub(in crate::analyzer::semantic) enum ScalarResolution {
    /// Every argument was a literal and the function is deterministic: the
    /// call collapsed to its value.
    Folded(TypedExpr),
    /// The resolved signature and the coerced arguments.
    Call(FunctionSignature, Vec<TypedExpr>),
}

impl<'a> AnalyzerContext<'a> {
    /// Resolve `name(args)` against the registry, coerce the arguments to the
    /// chosen signature and fold when possible.
    pub(in crate::analyzer::semantic) fn resolve_scalar_call(
        &self,
        name: &str,
        args: Vec<TypedExpr>,
    ) -> Result<ScalarResolution> {
        let arg_types: Vec<DataType> = args.iter().map(|a| a.data_type.clone()).collect();

        let signature = self
            .functions
            .resolve(name, &arg_types)
            .ok_or_else(|| AnalysisError::FunctionNotFound {
                name: name.to_string(),
                args: arg_types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            })?
            .clone();

        let mut coerced_args = Vec::with_capacity(args.len());
        for (arg, param_type) in args.into_iter().zip(&signature.params) {
            coerced_args.push(self.coerce_if_needed(arg, param_type)?);
        }

        if signature.is_deterministic
            && coerced_args
                .iter()
                .all(|a| matches!(a.expr, Expr::Literal(_)))
        {
            if let Some(folded) = self.try_constant_fold(name, &coerced_args)? {
                return Ok(ScalarResolution::Folded(folded));
            }
        }

        Ok(ScalarResolution::Call(signature, coerced_args))
    }

    /// Analyze a call to a named scalar function whose arguments are already
    /// typed — the entry point for the special AST forms.
    pub(in crate::analyzer::semantic) fn analyze_scalar_call(
        &self,
        name: &str,
        args: Vec<TypedExpr>,
    ) -> Result<TypedExpr> {
        if let Some(result) = self.analyze_variadic_scalar(name, &args)? {
            return Ok(result);
        }
        let (signature, args) = match self.resolve_scalar_call(name, args)? {
            ScalarResolution::Folded(folded) => return Ok(folded),
            ScalarResolution::Call(signature, args) => (signature, args),
        };
        let return_type = signature.return_type.clone();
        Ok(TypedExpr::new(
            Expr::Function {
                name: name.to_string(),
                args,
                signature,
                filter: None,
            },
            return_type,
        ))
    }
}
