// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Budget checks and usage accumulation (pure).

use crate::events::{OpOutcome, OpUsage};
use crate::record::{AgentRunRecord, OperationKind, RunUsage};

/// A budget that would be exceeded by starting the next operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exceeded {
    /// Which budget (`max_operations`, …).
    pub which: String,
    /// Its limit.
    pub limit: u64,
    /// Current use.
    pub used: u64,
}

fn over(which: &str, limit: Option<u64>, used: u64) -> Option<Exceeded> {
    match limit {
        Some(limit) if used >= limit => Some(Exceeded {
            which: which.into(),
            limit,
            used,
        }),
        _ => None,
    }
}

/// Check the budgets before starting an operation of `kind`.
///
/// `starts_turn` is true when this operation would open a new turn.
pub fn check_before_op(
    rec: &AgentRunRecord,
    kind: &OperationKind,
    starts_turn: bool,
    now_ms: u64,
) -> Option<Exceeded> {
    let b = &rec.budgets;
    let t = tree_used(rec);
    let u = &t;
    if starts_turn {
        if let Some(e) = over(
            "max_turns",
            b.max_turns.map(u64::from),
            u64::from(rec.counters.turn),
        ) {
            return Some(e);
        }
    }
    over("max_operations", b.max_operations, u.operations)
        .or_else(|| {
            if *kind == OperationKind::ModelTurn {
                over(
                    "max_model_calls",
                    b.max_model_calls.map(u64::from),
                    u64::from(u.model_calls),
                )
            } else {
                None
            }
        })
        .or_else(|| {
            over(
                "max_total_tokens",
                b.max_total_tokens,
                u.input_tokens + u.output_tokens,
            )
        })
        .or_else(|| {
            over(
                "max_wall_ms",
                b.max_wall_ms,
                now_ms.saturating_sub(rec.created_at_ms),
            )
        })
        .or_else(|| {
            over(
                "max_consecutive_op_failures",
                b.max_consecutive_op_failures.map(u64::from),
                u64::from(u.consecutive_op_failures),
            )
        })
}

/// What a run counts against its own budgets: its own usage, what finished
/// children consumed, and what live children still hold reserved. A parent
/// therefore cannot spend what it lent to a child, and a run tree as a whole
/// stays inside the root's budgets.
pub fn tree_used(rec: &AgentRunRecord) -> RunUsage {
    let mut t = rec.usage.tree_total();
    t.consecutive_op_failures = rec.usage.consecutive_op_failures;
    for link in rec.children.iter().filter(|l| l.is_live()) {
        let r = &link.reserved;
        t.operations += r.max_operations.unwrap_or(0);
        t.model_calls += r.max_model_calls.unwrap_or(0);
        t.input_tokens += r.max_total_tokens.unwrap_or(0);
    }
    t
}

/// What `rec` can still lend a new child, per budget: `None` = unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spare {
    /// Operations.
    pub operations: Option<u64>,
    /// Model calls.
    pub model_calls: Option<u32>,
    /// Tokens.
    pub tokens: Option<u64>,
    /// Wall time.
    pub wall_ms: Option<u64>,
}

/// The spare budget of `rec` at `now_ms`.
pub fn spare(rec: &AgentRunRecord, now_ms: u64) -> Spare {
    let t = tree_used(rec);
    let b = &rec.budgets;
    Spare {
        operations: b.max_operations.map(|l| l.saturating_sub(t.operations)),
        model_calls: b.max_model_calls.map(|l| l.saturating_sub(t.model_calls)),
        tokens: b
            .max_total_tokens
            .map(|l| l.saturating_sub(t.input_tokens + t.output_tokens)),
        wall_ms: b
            .max_wall_ms
            .map(|l| l.saturating_sub(now_ms.saturating_sub(rec.created_at_ms))),
    }
}

/// Fold one finished operation into `usage`.
pub fn accumulate(
    usage: &mut RunUsage,
    kind: &OperationKind,
    outcome: OpOutcome,
    op: Option<OpUsage>,
) {
    usage.operations += 1;
    match kind {
        OperationKind::ModelTurn => usage.model_calls += 1,
        OperationKind::ToolCall => usage.tool_calls += 1,
        _ => {}
    }
    if let Some(op) = op {
        usage.input_tokens += op.input_tokens;
        usage.output_tokens += op.output_tokens;
    }
    if outcome.is_failure() {
        usage.consecutive_op_failures += 1;
    } else if outcome != OpOutcome::Cancelled {
        usage.consecutive_op_failures = 0;
    }
}

/// Apply a resume's budget increase: every `Some` field replaces the old value.
pub fn raise(budgets: &mut crate::record::RunBudgets, inc: &crate::record::RunBudgets) {
    macro_rules! take {
        ($($f:ident),*) => { $( if inc.$f.is_some() { budgets.$f = inc.$f; } )* };
    }
    take!(
        max_turns,
        max_operations,
        max_model_calls,
        max_total_tokens,
        max_wall_ms,
        max_consecutive_op_failures,
        max_children,
        max_live_children,
        max_depth
    );
}
