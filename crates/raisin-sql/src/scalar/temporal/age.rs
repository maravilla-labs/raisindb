//! AGE([reference,] timestamp): the interval between two instants.
//!
//! `AGE(ts)` measures from today's midnight UTC, as PostgreSQL's one-argument
//! form measures from `current_date`. The result is an exact duration
//! (`398 days 04:00:00`), not PostgreSQL's symbolic `1 year 1 mon 3 days`:
//! RaisinDB's interval is a `chrono::Duration` with no month component.

use super::super::args::{arity, timestamp};
use super::super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use chrono::Utc;

pub(super) const KERNEL: Kernel = Kernel {
    name: "AGE",
    aliases: &[],
    category: KernelCategory::Temporal,
    signature: "AGE([reference,] timestamp) -> INTERVAL",
    deterministic: false,
    strict: true,
    func: age,
};

fn age(args: &[Literal]) -> KernelResult {
    arity("AGE", args, 1, 2)?;
    let (later, earlier) = if args.len() == 2 {
        (timestamp("AGE", args, 0)?, timestamp("AGE", args, 1)?)
    } else {
        (super::midnight(Utc::now()), timestamp("AGE", args, 0)?)
    };
    Ok(Literal::Interval(later - earlier))
}
