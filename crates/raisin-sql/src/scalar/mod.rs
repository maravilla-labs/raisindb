//! Scalar function kernels: the ONE implementation of every PostgreSQL-style
//! scalar function RaisinDB supports.
//!
//! A kernel is a pure function over [`Literal`] values. The analyzer's constant
//! folder and the execution engine's `SqlFunction` registry both call the same
//! kernel, so `LOWER('A')` folded at plan time and `LOWER(name)` evaluated per
//! row cannot drift apart — the recurring bug class in this codebase is two
//! mirrored code paths, and this module exists so scalar functions never grow
//! a second one.
//!
//! What lives here is only what is pure and WASM-compatible: no storage, no
//! row context, no auth. Functions that need the execution context (RESOLVE,
//! EMBEDDING, CURRENT_USER) stay in `raisin-sql-execution`.

mod args;
mod compare;
mod format;
mod hash;
mod math;
mod regexp;
mod string;
mod string_trim;
pub mod temporal;
mod typeof_fn;

#[cfg(test)]
mod tests;

use crate::analyzer::Literal;
use std::collections::HashMap;
use std::sync::LazyLock;

pub use args::KernelError;
pub use temporal::{format_interval, parse_timestamp};

/// Result of a kernel call.
pub type KernelResult = Result<Literal, KernelError>;

/// Which family a kernel belongs to; mirrors the execution-side categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelCategory {
    Math,
    String,
    Temporal,
    System,
}

/// One scalar function implementation.
pub struct Kernel {
    /// Canonical uppercase name.
    pub name: &'static str,
    /// Other names the same kernel answers to (`CEILING` for `CEIL`).
    pub aliases: &'static [&'static str],
    pub category: KernelCategory,
    /// Human-readable signature for error messages and completion.
    pub signature: &'static str,
    /// Same inputs always give the same output — safe to constant-fold.
    pub deterministic: bool,
    /// Any NULL argument yields NULL without calling the kernel.
    pub strict: bool,
    pub func: fn(&[Literal]) -> KernelResult,
}

static KERNELS: LazyLock<HashMap<&'static str, &'static Kernel>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    for kernel in all_kernels() {
        map.insert(kernel.name, kernel);
        for alias in kernel.aliases {
            map.insert(*alias, kernel);
        }
    }
    map
});

/// Every kernel, in registration order.
pub fn all_kernels() -> Vec<&'static Kernel> {
    let mut v: Vec<&'static Kernel> = Vec::new();
    v.extend(math::KERNELS.iter());
    v.extend(string::KERNELS.iter());
    v.extend(string_trim::KERNELS.iter());
    v.extend(hash::KERNELS.iter());
    v.extend(regexp::KERNELS.iter());
    v.extend(format::KERNELS.iter());
    v.extend(temporal::KERNELS.iter());
    v.extend(typeof_fn::KERNELS.iter());
    v
}

/// Look a kernel up by name (case-insensitive).
pub fn lookup(name: &str) -> Option<&'static Kernel> {
    KERNELS.get(name.to_uppercase().as_str()).copied()
}

/// Evaluate a kernel by name, applying strict NULL propagation.
///
/// Returns `None` when no such kernel exists, so callers can fall through to
/// their own dispatch.
pub fn evaluate(name: &str, args: &[Literal]) -> Option<KernelResult> {
    let kernel = lookup(name)?;
    Some(call(kernel, args))
}

/// Evaluate a resolved kernel, applying strict NULL propagation.
pub fn call(kernel: &Kernel, args: &[Literal]) -> KernelResult {
    if kernel.strict && args.iter().any(|a| matches!(a, Literal::Null)) {
        return Ok(Literal::Null);
    }
    (kernel.func)(args)
}

/// Constant-fold a call whose arguments are all literals.
///
/// Only deterministic kernels fold. A kernel error is NOT folded either: the
/// runtime will raise the same error for the same inputs, and folding it into
/// an analysis failure would turn a dead branch (`CASE WHEN false THEN LOG(0)`)
/// into a query that cannot even be planned.
pub fn fold(name: &str, args: &[Literal]) -> Option<Literal> {
    let kernel = lookup(name)?;
    if !kernel.deterministic {
        return None;
    }
    // An unbound `$1` is a Literal too, and CONCAT would happily render it
    // as the text "$1" — a placeholder is not a value.
    if args.iter().any(|a| matches!(a, Literal::Parameter(_))) {
        return None;
    }
    call(kernel, args).ok()
}
