// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! One transport-neutral entry point: `call(method, args)`.
//!
//! The HTTP routes, the function bindings (every runtime) and the SDKs all
//! go through here, so an agent tool, an external client and a server
//! function see exactly the same shapes and rules.
//!
//! Two things happen here and nowhere else:
//!
//! - **The grant.** When the caller hands in a `grant` (an agent run's roots,
//!   read from the run record by the transport — never from the model's
//!   arguments), the requested roots are narrowed by it: a root outside the
//!   grant is refused, and operations the grant does not give are removed.
//! - **Tool mode.** Arguments carrying `__raisin_context: {run_id,
//!   operation_id}` (what a tool call receives), or `envelope: true`, get a
//!   `raisin.tool-result/1` envelope back — errors included — and a mutating
//!   call with no idempotency key gets `run:<run_id>:<operation_id>`, so a
//!   replayed continuation can never apply twice.

use raisin_models::auth::AuthContext;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::envelope::*;
use super::read::DiffSide;
use super::root::Roots;
use super::*;

/// Every method name `call` accepts.
pub const METHODS: &[&str] = &[
    "stat",
    "read",
    "list",
    "diff",
    "watch",
    "dry_run",
    "propose",
    "get_changeset",
    "list_changesets",
    "commit",
    "discard",
    "apply",
    "fork_branch",
    "diff_branch",
    "merge_branch",
    "discard_branch",
];

#[derive(Debug, Default, Deserialize)]
struct ToolCtx {
    run_id: Option<String>,
    operation_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Common {
    #[serde(default)]
    roots: Option<Vec<WorkRoot>>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default, rename = "__raisin_context")]
    tool: Option<ToolCtx>,
    #[serde(default)]
    envelope: bool,
    #[serde(default)]
    operation_id: Option<String>,
}

/// A target given as an object or as a bare path string.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TargetArg {
    Path(String),
    Full(Target),
}

impl From<TargetArg> for Target {
    fn from(t: TargetArg) -> Self {
        match t {
            TargetArg::Path(p) => Target::path(&p),
            TargetArg::Full(t) => t,
        }
    }
}

fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> DevResult<Option<T>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| NodeDevError::invalid(format!("'{key}': {e}"))),
    }
}

fn req_arg<T: DeserializeOwned>(args: &Value, key: &str) -> DevResult<T> {
    arg(args, key)?.ok_or_else(|| NodeDevError::invalid(format!("'{key}' is required")))
}

fn target(args: &Value) -> DevResult<Target> {
    Ok(req_arg::<TargetArg>(args, "target")?.into())
}

fn to_json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Resolve `call`'s effective roots.
fn effective_roots(common: &Common, grant: Option<&[WorkRoot]>) -> DevResult<Vec<WorkRoot>> {
    let asked = match (&common.roots, &common.workspace) {
        (Some(r), _) if !r.is_empty() => r.clone(),
        (_, Some(ws)) => vec![WorkRoot::workspace(ws)],
        _ => match grant {
            Some(g) if !g.is_empty() => g.to_vec(),
            _ => {
                return Err(NodeDevError::invalid(
                    "'roots' (or 'workspace') is required",
                ))
            }
        },
    };
    let roots = Roots::new(asked)?;
    let roots = match grant {
        Some(g) if !g.is_empty() => roots.narrowed_by(g)?,
        _ => roots,
    };
    Ok(roots.all().to_vec())
}

fn changeset_request(
    args: &Value,
    roots: Vec<WorkRoot>,
    key: Option<String>,
) -> DevResult<ChangesetRequest> {
    Ok(ChangesetRequest {
        roots,
        ops: req_arg(args, "ops")?,
        idempotency_key: arg::<String>(args, "idempotency_key")?.or(key),
        message: arg(args, "message")?,
        primary: arg(args, "primary")?,
        kind: arg(args, "kind")?,
    })
}

/// Run one method. `grant` is an agent run's roots, when the call is made on
/// a run's behalf.
pub async fn call<S: Storage + TransactionalStorage>(
    svc: &NodeDevService<S>,
    scope: &DevScope,
    auth: &AuthContext,
    method: &str,
    args: Value,
    grant: Option<&[WorkRoot]>,
) -> DevResult<Value> {
    let common: Common = serde_json::from_value(args.clone())
        .map_err(|e| NodeDevError::invalid(format!("arguments: {e}")))?;
    let tool_mode = common.envelope || common.tool.is_some();
    let op_id = common
        .tool
        .as_ref()
        .and_then(|t| t.operation_id.clone())
        .or_else(|| common.operation_id.clone())
        .unwrap_or_else(|| format!("node-dev-{method}"));
    let derived_key = common
        .tool
        .as_ref()
        .and_then(|t| match (&t.run_id, &t.operation_id) {
            (Some(r), Some(o)) => Some(format!("run:{r}:{o}")),
            _ => None,
        });
    let result = run(
        svc,
        scope,
        auth,
        method,
        &args,
        &common,
        grant,
        derived_key,
        &op_id,
        tool_mode,
    )
    .await;
    match result {
        Ok(v) => Ok(v),
        Err(e) if tool_mode => Ok(to_json(&error_envelope(&op_id, &e))),
        Err(e) => Err(e),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run<S: Storage + TransactionalStorage>(
    svc: &NodeDevService<S>,
    scope: &DevScope,
    auth: &AuthContext,
    method: &str,
    args: &Value,
    common: &Common,
    grant: Option<&[WorkRoot]>,
    key: Option<String>,
    op_id: &str,
    tool: bool,
) -> DevResult<Value> {
    let roots = || effective_roots(common, grant);
    let read_out = |locs: Vec<NodeLocator>, payload: Value| {
        if tool {
            to_json(&read_envelope(op_id, &locs, payload))
        } else {
            payload
        }
    };
    let commit_out = |o: CommitOutcome, primary: usize, kind: &str| {
        if !tool {
            return to_json(&o);
        }
        match &o {
            CommitOutcome::Committed { receipt } => {
                to_json(&receipt_envelope(op_id, receipt, primary, kind))
            }
            CommitOutcome::Conflict {
                changeset_id,
                conflicts,
                digest,
            } => to_json(&conflict_envelope(op_id, changeset_id, conflicts, digest)),
        }
    };
    match method {
        "stat" => {
            let r = svc
                .stat(scope, auth, &Roots::new(roots()?)?, &target(args)?)
                .await?;
            Ok(read_out(
                r.locator.clone().into_iter().collect(),
                to_json(&r),
            ))
        }
        "read" => {
            let keys: Option<Vec<String>> = arg(args, "keys")?;
            let r = svc
                .read(
                    scope,
                    auth,
                    &Roots::new(roots()?)?,
                    &target(args)?,
                    keys.as_deref(),
                )
                .await?;
            Ok(read_out(vec![r.locator.clone()], to_json(&r)))
        }
        "list" => {
            let rs = Roots::new(roots()?)?;
            let t = match arg::<TargetArg>(args, "target")? {
                Some(t) => t.into(),
                None => Target {
                    workspace: Some(rs.first().workspace.clone()),
                    path: Some(rs.first().path.clone()),
                    node_id: None,
                },
            };
            let limit = arg::<usize>(args, "limit")?.unwrap_or(200).min(1000);
            let r = svc.list(scope, auth, &rs, &t, limit).await?;
            let locs = r
                .parent
                .iter()
                .cloned()
                .chain(r.children.iter().map(|c| c.locator.clone()))
                .collect();
            Ok(read_out(locs, to_json(&r)))
        }
        "diff" => {
            let from: DiffSide = arg(args, "from")?.unwrap_or_default();
            let to: DiffSide = arg(args, "to")?.unwrap_or_default();
            let r = svc
                .diff(
                    scope,
                    auth,
                    &Roots::new(roots()?)?,
                    &target(args)?,
                    &from,
                    &to,
                )
                .await?;
            Ok(read_out(
                r.from.iter().chain(r.to.iter()).cloned().collect(),
                to_json(&r),
            ))
        }
        "watch" => {
            let since: Option<String> = arg(args, "since")?;
            let limit = arg::<usize>(args, "limit")?.unwrap_or(100).min(1000);
            let r = svc
                .watch(scope, auth, &Roots::new(roots()?)?, since.as_deref(), limit)
                .await?;
            Ok(read_out(vec![], to_json(&r)))
        }
        "dry_run" => {
            let req = changeset_request(args, roots()?, None)?;
            let plan = svc.dry_run(scope, auth, &req).await?;
            Ok(read_out(
                plan.ops.iter().filter_map(|p| p.before.clone()).collect(),
                to_json(&plan),
            ))
        }
        "propose" => {
            let req = changeset_request(args, roots()?, key)?;
            let (rec, created) = svc.propose(scope, auth, req).await?;
            if tool {
                return Ok(to_json(&proposal_envelope(op_id, &rec)));
            }
            Ok(json!({ "changeset": rec, "created": created }))
        }
        "get_changeset" => {
            let rec = svc
                .get_changeset(scope, auth, &req_arg::<String>(args, "changeset_id")?)
                .await?;
            Ok(if tool {
                to_json(&proposal_envelope(op_id, &rec))
            } else {
                to_json(&rec)
            })
        }
        "list_changesets" => {
            let status: Option<ChangesetStatus> = arg(args, "status")?;
            let limit = arg::<usize>(args, "limit")?.unwrap_or(50).min(500);
            let list = svc.list_changesets(scope, auth, status, limit).await?;
            Ok(read_out(vec![], to_json(&list)))
        }
        "commit" => {
            let id: String = req_arg(args, "changeset_id")?;
            let digest: Option<String> = arg(args, "expected_digest")?;
            let o = svc.commit(scope, auth, &id, digest.as_deref()).await?;
            let rec = svc.load_record(scope, &id).await?;
            let (primary, kind) = rec
                .map(|r| {
                    (
                        r.request.primary.unwrap_or(0),
                        r.request.kind.unwrap_or_else(|| "node".into()),
                    )
                })
                .unwrap_or((0, "node".into()));
            Ok(commit_out(o, primary, &kind))
        }
        "discard" => {
            let rec = svc
                .discard(scope, auth, &req_arg::<String>(args, "changeset_id")?)
                .await?;
            Ok(read_out(vec![], to_json(&rec)))
        }
        "apply" => {
            let req = changeset_request(args, roots()?, key)?;
            let (primary, kind) = (
                req.primary.unwrap_or(0),
                req.kind.clone().unwrap_or_else(|| "node".into()),
            );
            let o = svc.apply(scope, auth, req).await?;
            Ok(commit_out(o, primary, &kind))
        }
        "fork_branch" => {
            let b = svc
                .fork_branch(scope, auth, &req_arg::<String>(args, "name")?)
                .await?;
            Ok(read_out(vec![], to_json(&b)))
        }
        "diff_branch" => {
            let base: String = arg(args, "base")?.unwrap_or_else(|| "main".into());
            let d = svc
                .diff_branch(scope, &Roots::new(roots()?)?, &base)
                .await?;
            Ok(read_out(vec![], to_json(&d)))
        }
        "merge_branch" => {
            let into: String = req_arg(args, "target")?;
            let message: Option<String> = arg(args, "message")?;
            let dry: bool = arg(args, "dry_run")?.unwrap_or(false);
            let m = svc
                .merge_branch(scope, auth, &into, message.as_deref(), dry)
                .await?;
            Ok(read_out(vec![], to_json(&m)))
        }
        "discard_branch" => {
            let deleted = svc.discard_branch(scope, auth).await?;
            Ok(read_out(vec![], json!({ "deleted": deleted })))
        }
        other => Err(NodeDevError::invalid(format!(
            "unknown method '{other}' (one of: {})",
            METHODS.join(", ")
        ))),
    }
}
