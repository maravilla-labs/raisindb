//! TYPEOF / PG_TYPEOF: the PostgreSQL type name of a value's RUNTIME type.
//!
//! Reports what the executor actually holds, so `TYPEOF(properties->>'n')`
//! says `text` even when the JSON had a number — that is what `->>` yields.

use super::args::{arity, describe};
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;

pub(super) static KERNELS: &[Kernel] = &[Kernel {
    name: "PG_TYPEOF",
    aliases: &["TYPEOF"],
    category: KernelCategory::System,
    signature: "PG_TYPEOF(any) -> TEXT",
    deterministic: true,
    strict: false,
    func: type_of,
}];

fn type_of(args: &[Literal]) -> KernelResult {
    arity("PG_TYPEOF", args, 1, 1)?;
    let name = match &args[0] {
        Literal::Null => "unknown",
        other => describe(other),
    };
    Ok(Literal::Text(name.to_string()))
}
