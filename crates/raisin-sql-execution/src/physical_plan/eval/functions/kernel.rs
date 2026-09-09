//! The scalar kernels of `raisin-sql`, exposed as execution-engine functions.
//!
//! A kernel is a pure function over `Literal` values. The analyzer's constant
//! folder already calls them; this adapter makes the executor call the same
//! ones per row, so `LOWER('A')` folded at plan time and `LOWER(name)`
//! evaluated per row cannot drift apart.
//!
//! Everything here is mechanical on purpose. There is no per-function file for
//! ABS or SUBSTR, because a second implementation is exactly what this bridge
//! exists to prevent — argument checking, NULL propagation and the result all
//! belong to the kernel. What stays hand-written in this crate is only what a
//! kernel cannot see: the row, the auth context, storage.

use super::registry::FunctionRegistry;
use super::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};
use raisin_sql::scalar::{self, Kernel, KernelCategory};

/// One registry entry for one kernel, under one of its names.
///
/// `name` is carried separately from `kernel.name` so an alias (`CEILING`,
/// `SUBSTRING`, `CHAR_LENGTH`) resolves to the same kernel without a second
/// implementation. The analyzer canonicalises names, so aliases normally never
/// reach here — they are registered for the callers that build an
/// `Expr::Function` directly.
struct KernelFunction {
    name: &'static str,
    kernel: &'static Kernel,
}

impl SqlFunction for KernelFunction {
    fn name(&self) -> &str {
        self.name
    }

    fn category(&self) -> FunctionCategory {
        match self.kernel.category {
            KernelCategory::Math => FunctionCategory::Numeric,
            KernelCategory::String => FunctionCategory::String,
            KernelCategory::Temporal => FunctionCategory::Temporal,
            KernelCategory::System => FunctionCategory::System,
        }
    }

    fn signature(&self) -> &str {
        self.kernel.signature
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(eval_expr(arg, row)?);
        }
        // `scalar::call` applies strict NULL propagation before the kernel, so
        // NULL handling is decided in one place for every function.
        scalar::call(self.kernel, &values).map_err(Error::Validation)
    }
}

/// Register every kernel, and every alias of every kernel.
pub fn register_functions(registry: &mut FunctionRegistry) {
    for kernel in scalar::all_kernels() {
        registry.register(Box::new(KernelFunction {
            name: kernel.name,
            kernel,
        }));
        for alias in kernel.aliases {
            registry.register(Box::new(KernelFunction {
                name: alias,
                kernel,
            }));
        }
    }
}
