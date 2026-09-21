/**
 * WHAT THIS RUN WROTE, AND WHETHER ITS EVIDENCE STILL HOLDS.
 *
 * The finalize gate (`finalize.js`) decides from what a task DECLARES: a task
 * that names a `build_target_path` needs a verification record, a task that
 * names none closes on the agent's word. Measured on the dev server
 * (2026-09-21, Studio Builder, round 2) that left three holes, all of them in
 * the routing of evidence rather than in the gate's decision:
 *
 *   1. A task that never declared a target was not a build task, so omitting
 *      `build_target_path` was enough to close one that had just created an
 *      automation (`success: true`).
 *   2. Evidence belonged to whichever task happened to name the artifact when
 *      the test ran. A separate "Test the function" task naming the same
 *      function was refused `unverified` five times, while the record for that
 *      exact function sat on its sibling "Draft" task.
 *   3. A record was forever. Re-drafting the function after its test left the
 *      old record in place, and the gate accepted it.
 *
 * All three are answered here from the run's OWN persisted tool calls — the
 * `raisin:AIToolCall` nodes and their `raisin:AIToolSingleCallResult` children
 * under the conversation — never from the model's prose or arguments alone. A
 * tool call is an ARTIFACT WRITE only when its RESULT says so:
 *
 *   - explicitly: `artifact: {workspace, path}` or `artifacts: [...]` on a
 *     successful result — the contract a write tool should adopt;
 *   - by the shapes the current write tools already return: a successful,
 *     non-dry-run result carrying a `path` and a write marker (`created`
 *     boolean, a non-empty `changed` list, `id` + `node_type`, or a
 *     `source_path`), or a compile result (`flow_path`) for the
 *     `automation_path` it was called with, unless it reports `unchanged`.
 *
 * A refused write (`success: false`), a dry run and a read are not writes.
 *
 * Kept free of anything but `finalize.js`'s predicates so `update-task`, which
 * is `execution_mode: inline` and runs on the tool-call path, can import it
 * without the agent runtime's module graph.
 */

import {
  FINALIZE_POLICY_VERIFIED,
  finalizePolicyOf,
  refToPath,
  taskBuildTarget,
  taskVerification,
  collectRunEvidence,
  serverStatusStatement,
  gateTerminalContent,
} from './finalize.js';

const str = (v) => (typeof v === 'string' && v.trim() ? v.trim() : '');
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);

/** Tool results and properties can arrive as JSON text from the SQL layer. */
function objectOf(value) {
  if (isObject(value)) return value;
  if (typeof value === 'string') {
    try {
      const parsed = JSON.parse(value);
      return isObject(parsed) ? parsed : null;
    } catch (_) {
      return null;
    }
  }
  return null;
}

/**
 * Milliseconds since the epoch, or NaN. The SQL layer returns `created_at` with
 * MICROseconds (`…T10:08:01.817600+00:00`) and a function's `new Date()` gives
 * milliseconds; not every engine's Date.parse accepts six fractional digits, so
 * they are cut to three first.
 */
export function timeOf(value) {
  if (!value) return NaN;
  const text = (typeof value === 'string' ? value : String(value))
    .trim()
    .replace(/^(\d{4}-\d{2}-\d{2}) (\d)/, '$1T$2')
    .replace(/(\.\d{3})\d+/, '$1');
  const t = Date.parse(text);
  return Number.isFinite(t) ? t : NaN;
}

const rowsOf = (result) => (Array.isArray(result) ? result : Array.isArray(result?.rows) ? result.rows : []);

/* ── artifact identity ─────────────────────────────────────────────────────
 *
 * One artifact is spelled several ways in the wild: `build_target_path` as the
 * bare workspace-relative path (`/x`) or workspace-prefixed (`/automations/x`),
 * a verification envelope carrying the task's own spelling in `raisin:path`,
 * and a write result naming the workspace separately. Two spellings name the
 * same artifact when their workspaces do not contradict each other and one of
 * their spellings coincide. */

export function artifactOf(workspace, path) {
  const p = str(path);
  if (!p) return null;
  return { workspace: str(workspace) || null, path: p };
}

function spellings(a) {
  const out = new Set([a.path]);
  if (a.workspace) {
    const prefix = `/${a.workspace}`;
    if (a.path.startsWith(`${prefix}/`)) out.add(a.path.slice(prefix.length));
    else out.add(`${prefix}${a.path}`);
  }
  return out;
}

export function sameArtifact(a, b) {
  if (!a || !b) return false;
  if (a.workspace && b.workspace && a.workspace !== b.workspace) return false;
  const sb = spellings(b);
  for (const s of spellings(a)) if (sb.has(s)) return true;
  return false;
}

export function describeArtifact(a) {
  return a ? (a.workspace ? `${a.workspace}:${a.path}` : a.path) : '';
}

/** The artifact a task's verification record is about, or null. */
export function evidenceArtifact(props) {
  const ref = props && props.verification_ref;
  const path = refToPath(ref);
  if (!path) return null;
  const ws = isObject(ref) ? ref['raisin:workspace'] : null;
  return artifactOf(ws, path);
}

/* ── the run's writes ──────────────────────────────────────────────────── */

/**
 * What a tool result says it WROTE, as a list of artifacts. Empty when the
 * call wrote nothing, was refused, or was a dry run.
 */
export function writesOfResult(result, args) {
  const r = objectOf(result);
  if (!r || r.success !== true || r.dry_run === true) return [];
  const a = objectOf(args) || {};
  const explicit = [];
  if (isObject(r.artifact)) explicit.push(r.artifact);
  if (Array.isArray(r.artifacts)) explicit.push(...r.artifacts.filter(isObject));
  if (explicit.length > 0) {
    return explicit.map((x) => artifactOf(x.workspace, x.path)).filter(Boolean);
  }
  const path = str(r.path);
  const marker =
    typeof r.created === 'boolean' ||
    (Array.isArray(r.changed) && r.changed.length > 0) ||
    (str(r.id) && str(r.node_type)) ||
    !!str(r.source_path);
  if (path && marker) return [artifactOf(r.workspace || a.workspace, path)];
  // A materialize of an automation that no longer exists only DISARMS
  // (`deleted: true`); there is nothing left for a record to be about.
  if (str(r.flow_path) && str(a.automation_path) && r.unchanged !== true && r.deleted !== true) {
    return [artifactOf(a.workspace || 'automations', a.automation_path)];
  }
  return [];
}

/**
 * Every tool call this conversation made, paired with its result, oldest
 * first, and the artifact writes among them. Two queries, no cast.
 *
 * `readable: false` when either query threw — the caller decides what an
 * unreadable run means, and for the gate it is never "nothing was written".
 */
export async function collectRunWrites(workspace, chatPath) {
  let callRows;
  let resultRows;
  try {
    callRows = rowsOf(await raisin.sql.query(
      `SELECT path, properties, created_at FROM "${workspace}"
        WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIToolCall'`,
      [chatPath],
    ));
    resultRows = rowsOf(await raisin.sql.query(
      `SELECT path, properties, created_at FROM "${workspace}"
        WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIToolSingleCallResult'`,
      [chatPath],
    ));
  } catch (e) {
    return { readable: false, calls: [], writes: [], error: (e && e.message) || String(e) };
  }

  const results = new Map();
  for (const row of resultRows) {
    const props = objectOf(row.properties) || {};
    const parent = str(row.path).split('/').slice(0, -1).join('/');
    const entry = { props, at: row.created_at || props.created_at || null };
    if (parent) results.set(parent, entry);
    if (str(props.tool_call_id)) results.set(`id:${props.tool_call_id}`, entry);
  }

  const calls = [];
  for (const row of callRows) {
    const props = objectOf(row.properties) || {};
    const res = results.get(str(row.path)) || results.get(`id:${str(props.tool_call_id)}`) || null;
    const args = objectOf(props.arguments) || {};
    const result = res ? objectOf(res.props.result) : null;
    const at = row.created_at || props.created_at || null;
    calls.push({
      path: str(row.path),
      parent: str(row.path).split('/').slice(0, -1).join('/'),
      status: str(props.status),
      has_result: !!res,
      tool: str(props.function_name) || str(res && res.props.function_name) || 'unknown',
      args,
      result,
      at,
      done_at: (res && res.at) || at,
    });
  }
  calls.sort((x, y) => (timeOf(x.at) || 0) - (timeOf(y.at) || 0));

  const writes = [];
  for (const call of calls) {
    for (const artifact of writesOfResult(call.result, call.args)) {
      writes.push({ ...artifact, tool: call.tool, at: call.at, done_at: call.done_at, call });
    }
  }
  return { readable: true, calls, writes };
}

const isUpdateTask = (call) => call.tool === 'update-task' || call.tool === 'update_task';
const accepted = (call) => !!(call.result && call.result.success === true && call.result.ignored !== true);

/**
 * WHOSE WRITE IS IT. Each write belongs to exactly one task, or to none:
 *   - the task a tool explicitly bound it to (`result.task.task_id`, as
 *     draft-function reports), else
 *   - the task most recently set in_progress before the write that had not
 *     been closed by then — the task the agent said it was working on, else
 *   - when NO task was open, the task the agent turned to NEXT: the first
 *     accepted in_progress or completed after the write. A write made before
 *     any task was started is the work of whichever task is picked up next;
 *     charging it to nobody let "create first, start the task afterwards"
 *     close that task on the agent's word (adversarial review, 2026-09-21).
 * A write with no owner at all (nothing was picked up after it) belongs to the
 * task being completed now — see `writesForTask`.
 *
 * Attribution is by the agent's own accepted update-task calls, as persisted;
 * a refused or ignored call moves nothing.
 */
export function writeOwners(run) {
  const owners = new Map();
  if (!run || !Array.isArray(run.writes)) return owners;
  const events = [];
  for (const call of run.calls || []) {
    if (!isUpdateTask(call) || !accepted(call) || !call.args.task_id) continue;
    events.push({ t: timeOf(call.at), task: call.args.task_id, status: call.args.status });
  }
  for (const w of run.writes) {
    const bound = w.call && w.call.result && isObject(w.call.result.task) ? str(w.call.result.task.task_id) : '';
    if (bound) {
      owners.set(w, bound);
      continue;
    }
    const t = timeOf(w.at);
    const open = new Map();
    let next = null;
    for (const e of events) {
      if (Number.isFinite(t) && Number.isFinite(e.t) && e.t > t) {
        if (!next && (e.status === 'in_progress' || e.status === 'completed')) next = e.task;
        continue;
      }
      if (e.status === 'in_progress') open.set(e.task, e.t);
      else if (['completed', 'failed', 'cancelled', 'pending'].includes(e.status)) open.delete(e.task);
    }
    let owner = null;
    let latest = -Infinity;
    for (const [task, at] of open) {
      if (at >= latest) {
        latest = at;
        owner = task;
      }
    }
    owners.set(w, owner || next);
  }
  return owners;
}

/**
 * The writes that belong to one task (see `writeOwners`), for the gate to
 * treat as that task's build targets whether or not it declared any. A write
 * nobody picked up after it is charged to the task being completed now: this
 * completion IS the next thing the agent turned to.
 */
export function writesForTask(run, taskId) {
  if (!run || !Array.isArray(run.writes) || !taskId) return [];
  const owners = writeOwners(run);
  return run.writes.filter((w) => {
    const owner = owners.get(w);
    return owner ? owner === taskId : true;
  });
}

/**
 * Tool calls made in the SAME assistant turn (`msgPath`, the tool calls'
 * parent) that have not produced a result yet. Tool calls from one turn run
 * in parallel, so a completion batched with a write can be decided before the
 * write's result exists; an in-flight call is not evidence that nothing was
 * written. update-task's own calls are excluded.
 */
export function inFlightSiblings(run, msgPath) {
  if (!run || !Array.isArray(run.calls) || !str(msgPath)) return [];
  return run.calls.filter((c) =>
    c.parent === msgPath && !c.has_result && !isUpdateTask(c) &&
    !['completed', 'failed', 'cancelled', 'error'].includes(c.status));
}

/**
 * The write that makes a verification record STALE: the artifact was written
 * again after the record was made. A record with no `verified_at` cannot be
 * dated, so any write to its artifact in this run makes it stale.
 */
export function staleningWrite(props, artifact, run) {
  if (!run || !Array.isArray(run.writes) || !artifact) return null;
  const verifiedAt = timeOf(props && props.verified_at);
  let latest = null;
  for (const w of run.writes) {
    if (!sameArtifact(w, artifact)) continue;
    const t = timeOf(w.done_at || w.at);
    if (Number.isFinite(verifiedAt) && Number.isFinite(t) && t <= verifiedAt) continue;
    if (!latest || (timeOf(w.done_at || w.at) || 0) > (timeOf(latest.done_at || latest.at) || 0)) latest = w;
  }
  return latest;
}

function staleReason(write, props) {
  return `the verification of ${describeArtifact(write)} is stale: ${write.tool} wrote it at ${write.done_at || write.at}` +
    `, after it was verified${props && props.verified_at ? ` at ${props.verified_at}` : ''}`;
}

/**
 * The freshest usable verification record for `artifact` in this conversation.
 * The task being closed is looked at first; a SIBLING task's record counts
 * too, because a record is about the artifact, not about which task happened
 * to name it when the proof was obtained. Returns the record's source, or the
 * reasons none qualified.
 */
export function findEvidence(artifact, tasks, run, ownPath) {
  const stale = [];
  let best = null;
  const ordered = [...tasks].sort((a, b) => (a.path === ownPath ? -1 : b.path === ownPath ? 1 : 0));
  for (const t of ordered) {
    const props = t.props || {};
    const v = taskVerification(props);
    if (!v || !props.verification_hash) continue;
    const about = evidenceArtifact(props);
    if (!sameArtifact(about, artifact)) continue;
    const w = staleningWrite(props, artifact, run);
    if (w) {
      stale.push(staleReason(w, props));
      continue;
    }
    if (!best || (timeOf(props.verified_at) || 0) > (timeOf(best.props.verified_at) || 0)) best = t;
  }
  return { source: best, stale };
}

/* ── supporting artifacts: verified THROUGH what depends on them ──────────
 *
 * Which artifact kinds have a verifier of their own:
 *   - an automation: `verify-automation` stamps the record;
 *   - a function: `execute-function` stamps it on a passing fixture run.
 * An AGENT has none. Nothing tests an agent in isolation, so a task that
 * called `upsert-agent` — the natural way to build an AI decider — could never
 * close (measured by review 2026-09-21). An agent built FOR an automation is a
 * SUPPORTING artifact: it is verified by a ready `verify-automation` of an
 * automation that calls it, because that verification resolves the agent,
 * checks it is a raisin:AIAgent, that it is switched on and that its output
 * schema carries what the step reads. It is not a bypass: the covering call
 * is read from the run's own persisted results, it must post-date the agent's
 * last write AND the automation's last write, and the automation itself must
 * hold a fresh record. An agent nothing verified still refuses. */

const AGENT_PATH = /^(?:\/functions)?\/agents\/[^/]+$/;
const toolIs = (tool, name) => str(tool).replace(/_/g, '-') === name;

/** 'agent' for an artifact only verifiable through a dependent, else null. */
export function supportingKind(artifact, write) {
  if (!artifact || (artifact.workspace && artifact.workspace !== 'functions')) return null;
  const call = write && write.call;
  if (call && toolIs(call.tool, 'upsert-agent')) return 'agent';
  if (call && call.result && str(call.result.node_type) === 'raisin:AIAgent') return 'agent';
  return AGENT_PATH.test(artifact.path) ? 'agent' : null;
}

/**
 * The latest finish time of any write in this run to `artifact`, or -Infinity.
 * A write that cannot be dated counts as +Infinity — no verification can be
 * shown to post-date it, the same reading `staleningWrite` gives it.
 */
function lastWriteAt(artifact, run) {
  let latest = -Infinity;
  for (const w of (run && run.writes) || []) {
    if (!sameArtifact(w, artifact)) continue;
    const t = timeOf(w.done_at || w.at);
    if (!Number.isFinite(t)) return Infinity;
    if (t > latest) latest = t;
  }
  return latest;
}

/**
 * The ready, non-dry-run `verify-automation` calls of this run, each with the
 * automation it verified and the dependencies its record resolved.
 */
export function dependentVerifications(run) {
  const out = [];
  for (const c of (run && run.calls) || []) {
    if (!toolIs(c.tool, 'verify-automation')) continue;
    const r = c.result;
    if (!r || r.success !== true || r.ready !== true || r.unverified === true || c.args.dry_run === true) continue;
    const deps = isObject(r.verification) && Array.isArray(r.verification.dependencies) ? r.verification.dependencies : [];
    const automation = artifactOf(c.args.workspace || 'automations', r.automation_path || c.args.automation_path);
    if (automation) out.push({ call: c, automation, deps });
  }
  return out;
}

/**
 * The dependent verification that covers a supporting artifact, or the
 * reasons none does. `{covered: {automation, source, at, level, hash}}` or
 * `{covered: null, reasons: [...]}`.
 */
export function supportingEvidence(artifact, tasks, run, kind = supportingKind(artifact)) {
  const reasons = [];
  if (!kind) return { covered: null, reasons };
  const agentAt = lastWriteAt(artifact, run);
  let best = null;
  for (const v of dependentVerifications(run)) {
    if (!v.deps.some((d) => isObject(d) && d.kind === kind && sameArtifact(artifactOf(d.workspace, d.path), artifact))) continue;
    const at = timeOf(v.call.at);
    if (!(Number.isFinite(at) && at >= agentAt && at >= lastWriteAt(v.automation, run))) {
      reasons.push(`the verification of ${describeArtifact(v.automation)} at ${v.call.at} predates a later write to it or to ${describeArtifact(artifact)}`);
      continue;
    }
    const found = findEvidence(v.automation, tasks, run, null);
    if (!found.source) {
      reasons.push(`${describeArtifact(v.automation)} was verified but no task holds a current record for it (name it as a task's build_target_path and verify again)`);
      continue;
    }
    if (!best || at > best.at) {
      const p = found.source.props;
      best = { automation: v.automation, source: found.source, at, verified_at: v.call.at, level: p.proof_level, hash: p.verification_hash };
    }
  }
  return best ? { covered: best, reasons: [] } : { covered: null, reasons: [...new Set(reasons)] };
}

/** How to verify a supporting artifact, for a refusal the model can act on. */
export function supportingHint(artifact) {
  return `${describeArtifact(artifact)} is an agent, and an agent has no verifier of its own: it is verified by ` +
    'verify-automation on an automation whose agent step calls it (materialize the automation, then verify it, after the agent\'s last write)';
}

/**
 * The record a task carries for a supporting artifact it closes on: about the
 * AGENT (so a later write to the agent makes it stale), at the automation's
 * proof level, and naming the automation and task it came from.
 */
export function supportingEvidenceFields(spelling, covered) {
  return {
    verification_ref: { 'raisin:ref': spelling.replace(/^\/functions(?=\/)/, ''), 'raisin:workspace': 'functions', 'raisin:path': spelling },
    verification_hash: `via:${describeArtifact(covered.automation)}:${covered.hash}`,
    proof_level: covered.level,
    verified_at: covered.verified_at,
    verification_source_task: covered.source.path,
    verification_via: describeArtifact(covered.automation),
  };
}

/** The readonly verification fields, copied from the task that holds them. */
export function evidenceFields(props) {
  const out = {};
  for (const k of ['verification_ref', 'verification_hash', 'proof_level', 'verified_at']) {
    if (props[k] !== undefined && props[k] !== null) out[k] = props[k];
  }
  return out;
}

/**
 * Completed tasks whose OWN record has gone stale since they closed. The
 * finalize gate counts them as unverified: a record the run has since
 * overwritten is not proof of what is there now.
 */
export function staleCompletedTasks(evidence, run) {
  const out = [];
  if (!evidence || !Array.isArray(evidence.tasks)) return out;
  for (const t of evidence.tasks) {
    const props = t.props || {};
    if ((props.status || 'pending') !== 'completed') continue;
    const target = taskBuildTarget(props);
    if (!target) continue;
    const about = evidenceArtifact(props);
    if (!about) continue;
    const w = staleningWrite(props, about, run);
    if (w) out.push({ path: t.path, title: props.title || t.path, target, stale: staleReason(w, props) });
  }
  return out;
}

/**
 * Every artifact this run WROTE that has no fresh verification record on any
 * task of the conversation. The terminal statement is about the run, not only
 * about the tasks that were closed: a write owned by a cancelled task, by no
 * task, or made with no plan at all is still an unverified artifact, and
 * without this the model's prose went out unannotated beside it.
 */
export function unverifiedWrites(evidence, run) {
  const out = [];
  if (!run || !Array.isArray(run.writes)) return out;
  const tasks = (evidence && Array.isArray(evidence.tasks)) ? evidence.tasks : [];
  const seen = [];
  for (const w of run.writes) {
    const artifact = artifactOf(w.workspace, w.path);
    // Deduplicated only within ONE workspace spelling: `upsert-agent` reports no
    // workspace, and a workspace-less agent write must not hide a write to
    // `automations:/agents/x` behind the agent's coverage (review 2026-09-21).
    if (!artifact || seen.some((a) => sameArtifact(a, artifact) && a.workspace === artifact.workspace)) continue;
    seen.push(artifact);
    if (findEvidence(artifact, tasks, run, null).source) continue;
    const kind = supportingKind(artifact, w);
    if (kind && supportingEvidence(artifact, tasks, run, kind).covered) continue;
    out.push({ path: `write:${describeArtifact(artifact)}`, title: `written by ${w.tool}`, target: describeArtifact(artifact), artifact });
  }
  return out;
}

/**
 * The terminal-turn gate of `finalize.js`, with the run's writes taken into
 * account: a completed task whose evidence the run has since overwritten is
 * UNVERIFIED. Same result shape as `gateTerminalContent`; an agent without the
 * policy is passed straight through to it.
 */
export async function gateTerminalContentWithRunWrites(workspace, chatPath, agentProps, content) {
  if (finalizePolicyOf(agentProps) !== FINALIZE_POLICY_VERIFIED) {
    return gateTerminalContent(workspace, chatPath, agentProps, content);
  }
  const evidence = await collectRunEvidence(workspace, chatPath);
  if (evidence.readable !== false) {
    const run = await collectRunWrites(workspace, chatPath);
    if (run.readable === false) {
      evidence.readable = false;
    } else {
      for (const s of staleCompletedTasks(evidence, run)) {
        if (!evidence.unverified.some((u) => u.path === s.path)) evidence.unverified.push(s);
      }
      for (const u of unverifiedWrites(evidence, run)) {
        if (!evidence.unverified.some((x) => sameArtifact(artifactOf(null, x.target), u.artifact))) evidence.unverified.push(u);
      }
    }
  }
  const { gated, statement } = serverStatusStatement(evidence);
  if (!statement) return { content, gated: false, evidence, statement: null };
  if (gated) {
    return {
      content: `${statement}\n\nThe agent's own summary of this turn is in the conversation; it is not a statement of readiness.`,
      gated: true,
      evidence,
      statement,
    };
  }
  return { content: `${statement}\n\n${content}`, gated: false, evidence, statement };
}
