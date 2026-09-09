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
    functions::{FunctionCategory, FunctionSignature},
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};
use crate::scalar::KernelCategory;

/// Map a kernel's family onto the analyzer's function category.
fn kernel_category(category: KernelCategory) -> FunctionCategory {
    match category {
        KernelCategory::Temporal => FunctionCategory::Temporal,
        KernelCategory::System => FunctionCategory::System,
        KernelCategory::Math | KernelCategory::String => FunctionCategory::Scalar,
    }
}

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

        // A scalar kernel answers first. The kernels are arity-flexible
        // (`ROUND(x)` and `ROUND(x, 2)`, `SUBSTR(x, 1)` and `SUBSTR(x, 1, 3)`)
        // and the fixed-arity registry cannot express that, so the signature is
        // synthesised from the call site against the kernel's own bounds.
        let signature = match crate::scalar::resolve_signature(name, &arg_types) {
            Some(Ok(kernel)) => FunctionSignature {
                name: kernel.name.to_string(),
                params: arg_types.clone(),
                return_type: kernel.return_type,
                is_deterministic: kernel.deterministic,
                category: kernel_category(kernel.category),
            },
            Some(Err(message)) => {
                return Err(AnalysisError::FunctionNotFound {
                    name: name.to_string(),
                    args: message,
                })
            }
            None => self
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
                .clone(),
        };

        let mut coerced_args = Vec::with_capacity(args.len());
        for (arg, param_type) in args.into_iter().zip(&signature.params) {
            coerced_args.push(self.coerce_if_needed(arg, param_type)?);
        }

        if signature.is_deterministic
            && coerced_args
                .iter()
                .all(|a| matches!(a.expr, Expr::Literal(_)))
        {
            if let Some(folded) = self.try_constant_fold(&signature.name, &coerced_args)? {
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
        let name = signature.name.clone();
        Ok(TypedExpr::new(
            Expr::Function {
                name,
                args,
                signature,
                filter: None,
            },
            return_type,
        ))
    }
}
