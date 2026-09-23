// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Child admission (pure): may this parent spawn this child, with which
//! budgets, which principal, which write roots and which context?
//!
//! Admission is ONE commit on the parent: `ChildSpawned`, a [`ChildLink`]
//! holding the reservation, and the full [`ChildPlan`] stored beside the parent
//! so a crash before the child exists is repaired from it, byte for byte.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::budget::{self, Spare};
use crate::child::{
    ChildLink, ContextSelection, Delegation, SpawnChild, MAX_CHILDREN_PER_RUN, MAX_CONTEXT_ITEMS,
    MAX_DEPTH, MAX_INLINE_BYTES,
};
use crate::domain::ReducerRef;
use crate::events::{CheckpointReason, ResultRef, RunEventKind};
use crate::ids::{Principal, PrincipalKind, RunId, SubjectRef};
use crate::lifecycle::{refusal, TransitionRefusal};
use crate::record::{AgentRunRecord, BudgetPolicy, RunBudgets};
use crate::state::{RunState, RunStatus};
use crate::tx::{Transition, Tx};

/// Everything needed to create (or re-create) the child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildPlan {
    /// The child's id.
    pub run_id: RunId,
    /// Parent.
    pub parent_run_id: RunId,
    /// Root of the tree.
    pub root_run_id: RunId,
    /// Depth.
    pub depth: u8,
    /// Subject.
    pub subject: SubjectRef,
    /// Principal.
    pub principal: Principal,
    /// Agent reference.
    pub agent_ref: Option<String>,
    /// Effective budgets.
    pub budgets: RunBudgets,
    /// Input (`{objective, context, input, parent_run_id, child_no}`).
    pub input: Value,
    /// Reducer, for a server-driven child.
    pub reducer: Option<ReducerRef>,
    /// Executor configuration (write roots, allowed tools, …).
    pub executor_config: Option<Value>,
    /// Delegation carried on the child record.
    pub delegation: Delegation,
    /// Create idempotency key.
    pub create_key: String,
}

/// Result key of a child's plan in the parent's results.
pub fn plan_key(child_no: u32) -> String {
    format!("spawn:{child_no}")
}

fn carve(
    which: &str,
    requested: Option<u64>,
    spare: Option<u64>,
    halve: bool,
) -> Result<Option<u64>, TransitionRefusal> {
    match spare {
        None => Ok(requested),
        Some(0) => refusal(
            &format!("child_budget_exceeded:{which}"),
            format!("the parent has no {which} left to lend"),
        ),
        Some(s) => Ok(Some(match requested {
            Some(r) => r.min(s),
            None if halve => s.div_ceil(2),
            None => s,
        })),
    }
}

fn child_budgets(
    parent: &AgentRunRecord,
    req: &SpawnChild,
    sp: Spare,
) -> Result<RunBudgets, TransitionRefusal> {
    let r = &req.budgets;
    Ok(RunBudgets {
        max_turns: r.max_turns,
        max_operations: carve("max_operations", r.max_operations, sp.operations, true)?,
        max_model_calls: carve(
            "max_model_calls",
            r.max_model_calls.map(u64::from),
            sp.model_calls.map(u64::from),
            true,
        )?
        .map(|v| v.min(u64::from(u32::MAX)) as u32),
        max_total_tokens: carve("max_total_tokens", r.max_total_tokens, sp.tokens, true)?,
        max_wall_ms: carve("max_wall_ms", r.max_wall_ms, sp.wall_ms, false)?,
        max_consecutive_op_failures: r
            .max_consecutive_op_failures
            .or(parent.budgets.max_consecutive_op_failures),
        max_children: r.max_children,
        max_live_children: r.max_live_children,
        max_depth: parent.budgets.max_depth,
        on_exceeded: req.on_exceeded.unwrap_or(BudgetPolicy::Fail),
    })
}

/// The part of the child's budgets reserved from the parent: only what the
/// parent itself is limited on.
fn reservation(parent: &AgentRunRecord, child: &RunBudgets) -> RunBudgets {
    let b = &parent.budgets;
    RunBudgets {
        max_operations: b.max_operations.and(child.max_operations),
        max_model_calls: b.max_model_calls.and(child.max_model_calls),
        max_total_tokens: b.max_total_tokens.and(child.max_total_tokens),
        ..RunBudgets::default()
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn within(child: &Value, parent: &Value) -> bool {
    if str_of(child, "workspace") != str_of(parent, "workspace") {
        return false;
    }
    let cp = str_of(child, "path").unwrap_or("/");
    let pp = str_of(parent, "path").unwrap_or("/").trim_end_matches('/');
    let path_ok = pp.is_empty() || cp == pp || cp.starts_with(&format!("{pp}/"));
    let ops_ok = match (child.get("ops"), parent.get("ops")) {
        (_, None) => true,
        (None, Some(_)) => true, // inherits the parent's ops below
        (Some(Value::Array(c)), Some(Value::Array(p))) => c.iter().all(|o| p.contains(o)),
        _ => false,
    };
    path_ok && ops_ok
}

/// Narrow the requested write roots to the parent's grant.
fn narrow_roots(
    parent_roots: Option<&Vec<Value>>,
    requested: &[Value],
) -> Result<Vec<Value>, TransitionRefusal> {
    let mut out = Vec::with_capacity(requested.len());
    for root in requested {
        if str_of(root, "workspace").is_none_or(str::is_empty) {
            return refusal("invalid_write_scope", "every write root needs a workspace");
        }
        let mut root = root.clone();
        if let Some(parents) = parent_roots {
            let Some(p) = parents.iter().find(|p| within(&root, p)) else {
                return refusal(
                    "write_scope_not_within_parent",
                    format!("write root {root} is outside the parent's grant"),
                );
            };
            if root.get("ops").is_none() {
                if let (Some(obj), Some(ops)) = (root.as_object_mut(), p.get("ops")) {
                    obj.insert("ops".into(), ops.clone());
                }
            }
        }
        out.push(root);
    }
    Ok(out)
}

fn executor_config(
    parent: &AgentRunRecord,
    req: &SpawnChild,
) -> Result<Option<Value>, TransitionRefusal> {
    let mut cfg: Map<String, Value> = match &parent.executor_config {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    if let Some(Value::Object(extra)) = &req.executor_config {
        for (k, v) in extra {
            if k != "node_dev" {
                cfg.insert(k.clone(), v.clone());
            }
        }
    }
    let parent_roots = parent
        .executor_config
        .as_ref()
        .and_then(|c| c.pointer("/node_dev/roots"))
        .and_then(Value::as_array);
    if let Some(requested) = &req.objective.allowed_writes {
        let roots = narrow_roots(parent_roots, requested)?;
        let node_dev = cfg.entry("node_dev").or_insert_with(|| json!({}));
        if let Some(obj) = node_dev.as_object_mut() {
            obj.insert("roots".into(), Value::Array(roots));
        } else {
            *node_dev = json!({ "roots": roots });
        }
    }
    if !req.objective.allowed_tools.is_empty() {
        cfg.insert("allowed_tools".into(), json!(req.objective.allowed_tools));
    }
    Ok((!cfg.is_empty()).then_some(Value::Object(cfg)))
}

fn bounded(v: &Value, what: &str) -> Result<(), TransitionRefusal> {
    if serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX) > MAX_INLINE_BYTES {
        return refusal(
            "context_too_large",
            format!("{what} exceeds {MAX_INLINE_BYTES} bytes"),
        );
    }
    Ok(())
}

/// Resolve the context selection into what the child receives. A snapshot
/// is a REFERENCE to a parent checkpoint — written now when none is named.
fn context(tx: &mut Tx, sel: &ContextSelection) -> Result<Value, TransitionRefusal> {
    Ok(match sel {
        ContextSelection::None => json!({ "mode": "none" }),
        ContextSelection::RecentTurns { turns, items } => {
            let cap = (*turns as usize).min(MAX_CONTEXT_ITEMS);
            if items.len() > cap {
                return refusal(
                    "context_too_large",
                    format!("recent_turns carries {} items, bound is {cap}", items.len()),
                );
            }
            let v = json!({ "mode": "recent_turns", "turns": turns, "items": items });
            bounded(&v, "recent_turns context")?;
            v
        }
        ContextSelection::Snapshot {
            checkpoint_no,
            data,
        } => {
            let no = match checkpoint_no {
                Some(n) if *n >= 1 && *n <= tx.rec.counters.checkpoint => *n,
                Some(n) => {
                    return refusal(
                        "unknown_checkpoint",
                        format!("parent has no checkpoint {n}"),
                    )
                }
                None => {
                    tx.checkpoint(CheckpointReason::Delegation, None);
                    tx.rec.counters.checkpoint
                }
            };
            let v = json!({
                "mode": "snapshot", "parent_run_id": tx.rec.run_id, "checkpoint_no": no,
                "domain_state_rev": tx.rec.domain.as_ref().map(|d| d.state_rev), "data": data,
            });
            bounded(&v, "snapshot context")?;
            v
        }
    })
}

/// Admit a child of `parent` under id `child_id`.
pub fn apply_spawn(
    parent: &AgentRunRecord,
    req: &SpawnChild,
    child_id: RunId,
    now: u64,
) -> Result<(Transition, ChildPlan), TransitionRefusal> {
    match parent.state {
        RunState::Terminal { .. } | RunState::Cancelling { .. } => {
            return refusal(
                "parent_not_live",
                format!("parent is {}", parent.state.status().as_str()),
            )
        }
        _ => {}
    }
    if req.objective.title.trim().is_empty() {
        return refusal("invalid_objective", "an objective needs a title");
    }
    let b = &parent.budgets;
    let spawned = parent.children.len();
    if spawned >= MAX_CHILDREN_PER_RUN
        || b.max_children
            .is_some_and(|m| spawned as u64 >= u64::from(m))
    {
        return refusal(
            "child_budget_exceeded:max_children",
            "no more children may be spawned",
        );
    }
    let live = parent.children.iter().filter(|l| l.is_live()).count() as u64;
    if b.max_live_children.is_some_and(|m| live >= u64::from(m)) {
        return refusal(
            "child_budget_exceeded:max_live_children",
            "too many live children",
        );
    }
    let depth = parent.depth + 1;
    if depth > b.max_depth.unwrap_or(MAX_DEPTH).min(MAX_DEPTH) {
        return refusal("max_depth", format!("a child at depth {depth} is too deep"));
    }
    let budgets = child_budgets(parent, req, budget::spare(parent, now))?;
    let executor_config = executor_config(parent, req)?;
    let user = match parent.principal.kind {
        PrincipalKind::User => Some(parent.principal.id.clone()),
        _ => parent.principal.on_behalf_of.clone(),
    };
    let principal = match (&req.as_agent, user) {
        (Some(agent), user) => Principal {
            kind: PrincipalKind::Agent,
            id: agent.clone(),
            on_behalf_of: user,
        },
        _ => parent.principal.clone(),
    };

    let mut tx = Tx::new(parent, now);
    let ctx = context(&mut tx, &req.objective.context)?;
    tx.rec.counters.child += 1;
    let child_no = tx.rec.counters.child;
    let reserved = reservation(parent, &budgets);
    let subject = req.subject.clone().unwrap_or_else(|| SubjectRef {
        workspace: parent.subject.workspace.clone(),
        path: format!("{}#child-{child_no}", parent.subject.path),
        node_id: None,
    });
    let mut objective = req.objective.clone();
    // The context payload travels in `input.context`; the stored objective
    // keeps only the selection mode, so the record stays small.
    objective.context = match &objective.context {
        ContextSelection::RecentTurns { turns, .. } => ContextSelection::RecentTurns {
            turns: *turns,
            items: Vec::new(),
        },
        ContextSelection::Snapshot { .. } => ContextSelection::Snapshot {
            checkpoint_no: ctx
                .get("checkpoint_no")
                .and_then(Value::as_u64)
                .map(|n| n as u32),
            data: None,
        },
        other => other.clone(),
    };
    let plan = ChildPlan {
        run_id: child_id.clone(),
        parent_run_id: parent.run_id.clone(),
        root_run_id: parent
            .root_run_id
            .clone()
            .unwrap_or_else(|| parent.run_id.clone()),
        depth,
        subject,
        principal,
        agent_ref: req.agent_ref.clone().or_else(|| parent.agent_ref.clone()),
        budgets: budgets.clone(),
        input: json!({
            "objective": req.objective, "context": ctx, "input": req.input,
            "parent_run_id": parent.run_id, "child_no": child_no,
        }),
        reducer: req.reducer.clone(),
        executor_config,
        delegation: Delegation {
            objective,
            child_no,
            spawn_key: req.spawn_key.clone(),
            handback_delivered: false,
        },
        create_key: format!("child:{}:{child_no}", parent.run_id),
    };
    tx.push(RunEventKind::ChildSpawned {
        child_run_id: child_id.clone(),
        child_no,
        title: req.objective.title.clone(),
        reserved: reserved.clone(),
        depth,
    });
    tx.rec.children.push(ChildLink {
        child_no,
        run_id: child_id,
        title: req.objective.title.clone(),
        spawn_key: req.spawn_key.clone(),
        status: RunStatus::Queued,
        reserved,
        delivered: false,
        result_key: None,
        usage: None,
    });
    let bytes = serde_json::to_vec(&plan).unwrap_or_default();
    let r = ResultRef {
        key: plan_key(child_no),
        bytes: bytes.len() as u64,
        content_type: "application/json".into(),
    };
    tx.results.push((r, bytes));
    Ok((tx.finish(), plan))
}
