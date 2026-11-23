/**
 * Delegation as CHILD RUNS — the pure half: names, limits, the typed spawn
 * specification, grant narrowing, the child's brief, the objective core
 * stores on the child, and the acceptance a parent reads.
 *
 * PURE: no `raisin.*`, no clock, no randomness (the generic reducer imports
 * pieces of it). Everything that talks to the runtime lives in
 * `delegation-core.js`; core itself owns lineage, the hand-back, the mailbox
 * and the cascade (`raisin.agentRuns.spawnChild` and friends).
 *
 * The model (ADR "Delegation"):
 * - a child is an ordinary AgentRun of an installed agent, acting for the
 *   same user as its parent, with its own conversation as transcript;
 * - it receives a TYPED objective, selected context (`none`, bounded
 *   `recent` turns, or a structured `snapshot`), narrowed tool and write
 *   grants, expected artifacts, acceptance checks, and a budget;
 * - the parent can inspect, message, steer, wait for (durable mailbox) and
 *   interrupt it; parallel children only for independent work.
 */

import { digestOf, safeName, shortRun, normToolName } from './run-names.js';

/** Function paths of the delegation surface. */
export const DELEGATION_FUNCTIONS = Object.freeze({
  spawn: '/lib/raisin/ai/spawn-agent',
  inspect: '/lib/raisin/ai/inspect-agent',
  message: '/lib/raisin/ai/message-agent',
  wait: '/lib/raisin/ai/wait-for-agents',
  interrupt: '/lib/raisin/ai/interrupt-agent',
  legacyDelegate: '/lib/raisin/ai/delegate-task',
  legacyStatus: '/lib/raisin/ai/get-delegation-status',
});

/** Every delegation tool a model may be offered. */
export const DELEGATION_TOOL_PATHS = Object.freeze([
  DELEGATION_FUNCTIONS.spawn, DELEGATION_FUNCTIONS.inspect, DELEGATION_FUNCTIONS.message,
  DELEGATION_FUNCTIONS.wait, DELEGATION_FUNCTIONS.interrupt,
  DELEGATION_FUNCTIONS.legacyDelegate, DELEGATION_FUNCTIONS.legacyStatus,
]);

export const CONTEXT_MODES = Object.freeze(['none', 'recent', 'snapshot']);

/** Hard limits; an agent's `delegation` config may only narrow them. */
export const LIMITS = Object.freeze({
  max_depth: 2,
  max_parallel: 4,
  max_children: 12,
  recent_turns: 20,
  recent_chars: 1500,
  objective_chars: 4000,
  context_chars: 12000,
});

/** Default delegation policy of an agent without a `delegation` block. */
export const DEFAULT_POLICY = Object.freeze({
  max_depth: 1,
  max_parallel: 2,
  max_children: 6,
  allowed_agents: null,
});

/** Child budgets: small, and a child that overruns FAILS (a paused child would park its parent). */
export const DEFAULT_CHILD_BUDGETS = Object.freeze({
  max_model_calls: 20,
  max_operations: 80,
  max_consecutive_op_failures: 5,
  max_wall_ms: 20 * 60 * 1000,
  on_exceeded: 'fail',
});
const BUDGET_CEILING = Object.freeze({
  max_model_calls: 60,
  max_operations: 300,
  max_consecutive_op_failures: 8,
  max_wall_ms: 60 * 60 * 1000,
  max_total_tokens: 2000000,
});

export const TERMINAL_STATUSES = new Set(['completed', 'failed', 'stopped']);

const str = (v) => (typeof v === 'string' ? v.trim() : '');
const int = (v, lo, hi, dflt) => {
  const n = Number(v);
  if (!Number.isFinite(n)) return dflt;
  return Math.min(Math.max(Math.floor(n), lo), hi);
};

/** An error the envelope classifies as `invalid_input`. */
export function invalid(message) {
  return Object.assign(new Error(message), { error_class: 'invalid_input' });
}

/** An agent's effective delegation policy (config can only narrow the limits). */
export function policyOf(agentProps) {
  const cfg = agentProps && typeof agentProps.delegation === 'object' && agentProps.delegation ? agentProps.delegation : {};
  const allowed = Array.isArray(cfg.allowed_agents)
    ? cfg.allowed_agents.map((a) => agentPath(a)).filter(Boolean)
    : null;
  return {
    max_depth: int(cfg.max_depth, 0, LIMITS.max_depth, DEFAULT_POLICY.max_depth),
    max_parallel: int(cfg.max_parallel, 1, LIMITS.max_parallel, DEFAULT_POLICY.max_parallel),
    max_children: int(cfg.max_children, 1, LIMITS.max_children, DEFAULT_POLICY.max_children),
    allowed_agents: allowed,
  };
}

/** `/agents/x`, `functions:/agents/x` or a ref envelope → `/agents/x` (null when not an agent path). */
export function agentPath(ref) {
  let p = ref;
  if (ref && typeof ref === 'object') p = ref['raisin:path'] || ref.path;
  p = str(p);
  const idx = p.indexOf(':/');
  if (idx > 0) p = p.slice(idx + 1);
  return /^\/agents\/[a-z0-9][a-z0-9_-]*$/i.test(p) ? p : null;
}

/** The agent's name (its home folder in the `ai` workspace). */
export function agentSlug(path) {
  return String(path || '').split('/')[2] || null;
}

/** Normalize a model's objective: a string or `{goal, deliverable?, done_when?, constraints?}`. */
export function normalizeObjective(raw) {
  if (typeof raw === 'string') {
    const goal = raw.trim();
    if (!goal) throw invalid('objective is required');
    return { goal: goal.slice(0, LIMITS.objective_chars) };
  }
  if (!raw || typeof raw !== 'object') throw invalid('objective is required: {goal, deliverable?, done_when?, constraints?}');
  const goal = str(raw.goal || raw.objective || raw.title);
  if (!goal) throw invalid('objective.goal is required');
  const out = { goal: goal.slice(0, LIMITS.objective_chars) };
  if (str(raw.deliverable)) out.deliverable = str(raw.deliverable).slice(0, 1000);
  if (str(raw.done_when)) out.done_when = str(raw.done_when).slice(0, 1000);
  if (Array.isArray(raw.constraints)) out.constraints = raw.constraints.map(str).filter(Boolean).slice(0, 10);
  return out;
}

function normLocator(x, label) {
  if (!x || typeof x !== 'object') throw invalid(`${label} must be an object with workspace and path`);
  const workspace = str(x.workspace);
  const path = str(x.path || x.path_prefix);
  if (!workspace || !path.startsWith('/')) throw invalid(`${label} needs a workspace and an absolute path`);
  return { workspace, path };
}

/** Write grants: `[{workspace, path}]` subtree prefixes. Empty array = read-only child. */
export function normalizeWrites(raw) {
  if (raw === undefined || raw === null) return null;
  if (!Array.isArray(raw)) throw invalid('writes must be a list of {workspace, path} subtrees');
  return raw.slice(0, 20).map((w, i) => normLocator(w, `writes[${i}]`));
}

/** Whether `workspace:path` lies inside a write scope (null scope = unrestricted). */
export function inWriteScope(scope, workspace, path) {
  if (!Array.isArray(scope)) return true;
  const p = String(path || '');
  return scope.some((s) => s.workspace === workspace
    && (p === s.path || s.path === '/' || p.startsWith(`${s.path.replace(/\/$/, '')}/`)));
}

const CHECK_KINDS = new Set(['node_exists', 'property_equals', 'artifact_written', 'outcome_is']);

/** Acceptance checks the hand-back evaluates on durable state. */
export function normalizeChecks(raw) {
  if (raw === undefined || raw === null) return [];
  if (!Array.isArray(raw)) throw invalid('checks must be a list');
  return raw.slice(0, 20).map((c, i) => {
    const kind = str(c && c.kind);
    if (!CHECK_KINDS.has(kind)) throw invalid(`checks[${i}].kind must be one of ${[...CHECK_KINDS].join(', ')}`);
    if (kind === 'outcome_is') return { kind, value: str(c.value) || 'succeeded' };
    const loc = normLocator(c, `checks[${i}]`);
    if (kind === 'property_equals') {
      if (!str(c.property)) throw invalid(`checks[${i}].property is required`);
      return { kind, ...loc, property: str(c.property), value: c.value === undefined ? null : c.value };
    }
    return { kind, ...loc };
  });
}

/** Expected artifacts: `[{workspace, path, kind?, description?}]`. */
export function normalizeExpected(raw) {
  if (raw === undefined || raw === null) return [];
  if (!Array.isArray(raw)) throw invalid('expected_artifacts must be a list');
  return raw.slice(0, 20).map((a, i) => ({
    ...normLocator(a, `expected_artifacts[${i}]`),
    ...(str(a.kind) ? { kind: str(a.kind) } : {}),
    ...(str(a.description) ? { description: str(a.description).slice(0, 300) } : {}),
  }));
}

/**
 * The budgets a CONFIGURED agent runs with at top level when its
 * `run_budgets` leave a limit out (run-entry.js `DEFAULT_RUN_BUDGETS`).
 */
export const AGENT_RUN_DEFAULTS = Object.freeze({
  max_model_calls: 60,
  max_operations: 300,
  max_consecutive_op_failures: 8,
  max_wall_ms: 60 * 60 * 1000,
});

const ON_EXCEEDED = new Set(['pause', 'fail']);

/**
 * A child's budgets (core's `on_exceeded` travels inside).
 *
 * Without `own` (an agent that declares no `run_budgets`, or a copy of the
 * calling agent): small defaults, the requested values clamped to the engine
 * ceiling, and an overrun FAILS.
 *
 * With `own` (the child agent's `run_budgets`): the child starts from the
 * budgets it runs with at top level, a requested value may LOWER a limit the
 * agent declares but never raise it, nothing exceeds the engine ceiling, and
 * `on_exceeded` is the agent's own (`fail` may still be requested; `pause`
 * cannot be, since a paused child holds its parent's wait).
 */
export function childBudgets(raw, own = null) {
  const cfg = own && typeof own === 'object' && !Array.isArray(own) ? own : null;
  const b = cfg ? { ...AGENT_RUN_DEFAULTS } : { ...DEFAULT_CHILD_BUDGETS };
  delete b.on_exceeded;
  const declared = new Set();
  if (cfg) {
    const ownSrc = { ...cfg };
    if (ownSrc.max_wall_s !== undefined && ownSrc.max_wall_ms === undefined) ownSrc.max_wall_ms = Number(ownSrc.max_wall_s) * 1000;
    for (const k of Object.keys(BUDGET_CEILING)) {
      if (ownSrc[k] === undefined || ownSrc[k] === null) continue;
      b[k] = int(ownSrc[k], 1, BUDGET_CEILING[k], b[k]);
      declared.add(k);
    }
  }
  const src = raw && typeof raw === 'object' ? { ...raw } : {};
  if (src.max_wall_s !== undefined) src.max_wall_ms = Number(src.max_wall_s) * 1000;
  for (const k of Object.keys(BUDGET_CEILING)) {
    const v = src[k] !== undefined ? src[k] : b[k];
    if (v === undefined) continue;
    const n = int(v, 1, BUDGET_CEILING[k], b[k]);
    b[k] = declared.has(k) ? Math.min(n, b[k]) : n;
  }
  const ownPolicy = cfg && ON_EXCEEDED.has(cfg.on_exceeded) ? cfg.on_exceeded : 'fail';
  b.on_exceeded = src.on_exceeded === 'fail' ? 'fail' : ownPolicy;
  return b;
}

// ── The child's write grant (core: `executor_config.node_dev.roots`) ─────────

const OPS = Object.freeze(['read', 'create', 'update', 'delete', 'move', 'copy']);

function normPath(p) {
  const s = str(p) || '/';
  const t = s.length > 1 ? s.replace(/\/+$/, '') : s;
  return t.startsWith('/') ? t : `/${t}`;
}

function within(path, root) {
  return root === '/' || path === root || path.startsWith(`${root}/`);
}

/** Ops both roots allow (`undefined` = every op). */
function opsBoth(a, b) {
  if (!Array.isArray(a) && !Array.isArray(b)) return undefined;
  const x = Array.isArray(a) ? a : OPS;
  const y = Array.isArray(b) ? b : OPS;
  return OPS.filter((o) => x.includes(o) && y.includes(o));
}

/** An agent's `node_dev.roots`, cleaned (`null` when it declares none). */
export function agentRoots(agentProps) {
  const roots = agentProps && agentProps.node_dev && agentProps.node_dev.roots;
  if (!Array.isArray(roots)) return null;
  const clean = roots
    .filter((r) => r && typeof r === 'object' && str(r.workspace))
    .map((r) => ({
      workspace: str(r.workspace),
      path: normPath(r.path),
      ...(Array.isArray(r.ops) ? { ops: OPS.filter((o) => r.ops.includes(o)) } : {}),
    }));
  return clean.length ? clean : null;
}

/**
 * The part of roots `a` that roots `b` also cover: the deeper of two nested
 * paths, with the ops both allow. A pair that shares no ops is dropped.
 */
export function intersectRoots(a, b) {
  const out = [];
  const seen = new Set();
  for (const x of a || []) {
    for (const y of b || []) {
      if (x.workspace !== y.workspace) continue;
      const xp = normPath(x.path);
      const yp = normPath(y.path);
      let path = null;
      if (within(xp, yp)) path = xp;
      else if (within(yp, xp)) path = yp;
      if (!path) continue;
      const ops = opsBoth(x.ops, y.ops);
      if (Array.isArray(ops) && !ops.length) continue;
      const root = { workspace: x.workspace, path, ...(ops ? { ops } : {}) };
      const id = JSON.stringify(root);
      if (seen.has(id)) continue;
      seen.add(id);
      out.push(root);
    }
  }
  return out;
}

/**
 * The write grant of a child of a CONFIGURED agent that declares
 * `node_dev.roots`: its own roots, narrowed by the parent's grant (when the
 * parent has one) and by the model's `writes` (when given). Outside the
 * requested writes the child keeps READ access to its own roots, so it can
 * still look around the way it does at top level; `writes: []` makes every
 * root read-only. Never wider than the agent's own configuration.
 *
 * Returns `{ roots, write_scope }` (`write_scope` only when `writes` was
 * given: the subtrees the child may change). Throws when nothing is left: core reads
 * an empty grant as NO grant (unconfined), so an empty result is refused
 * here instead of being handed on.
 */
export function childGrant({ own, parent = null, writes = null }) {
  let base = own;
  if (Array.isArray(parent) && parent.length) {
    base = intersectRoots(own, parent.map((r) => ({ ...r, path: normPath(r.path) })));
    if (!base.length) {
      throw Object.assign(new Error(`The delegating run may not grant this agent any of its working roots (${own.map((r) => `${r.workspace}:${r.path}`).join(', ')}).`), { error_class: 'permission_denied' });
    }
  }
  if (!Array.isArray(writes)) {
    return { roots: base, write_scope: null };
  }
  const writable = writes.length ? intersectRoots(base, writes.map((w) => ({ workspace: w.workspace, path: normPath(w.path) }))) : [];
  if (writes.length && !writable.some((r) => !Array.isArray(r.ops) || r.ops.some((o) => o !== 'read'))) {
    throw invalid(`writes lie outside what this agent may write: ${base.map((r) => `${r.workspace}:${r.path}`).join(', ')}.`);
  }
  const readOnly = base
    .map((r) => ({ ...r, ops: opsBoth(r.ops, ['read']) }))
    .filter((r) => r.ops.length);
  const roots = [...writable, ...readOnly.filter((r) => !writable.some((w) => JSON.stringify(w) === JSON.stringify(r)))];
  if (!roots.length) {
    throw Object.assign(new Error('This agent has no root it may read; it cannot run as a read-only child.'), { error_class: 'permission_denied' });
  }
  return { roots, write_scope: writable.map((r) => ({ workspace: r.workspace, path: r.path })) };
}

// ── What a child asks for ────────────────────────────────────────────────────

const clip = (v, n) => (typeof v === 'string' ? v.slice(0, n) : v);

/**
 * The open requests of a child's run record (core's `PendingRequest`s, in
 * every state that can hold them), as a parent reads them. `needs_person`
 * marks what only a person can answer (an approval, a question).
 */
export function openRequestsOf(rec) {
  const st = (rec && rec.state) || {};
  const open = Array.isArray(st.open) ? st.open
    : st.activity && Array.isArray(st.activity.open) ? st.activity.open : [];
  return open.slice(0, 10).map((r) => {
    const k = (r && r.kind) || {};
    const base = { request_id: r.request_id, kind: k.kind || 'unknown', ...(r.expires_at_ms ? { expires_at_ms: r.expires_at_ms } : {}) };
    switch (k.kind) {
      case 'approval':
        return {
          ...base, needs_person: true, summary: clip(k.summary, 2000),
          ...(k.scope ? { scope: k.scope } : {}),
          subject_digest: k.subject_digest,
          ...(k.changes !== undefined && k.changes !== null ? { changes: boundedJson(k.changes, 4000) } : {}),
        };
      case 'input':
        return {
          ...base, needs_person: true, prompt: clip(k.prompt, 2000),
          ...(Array.isArray(k.choices) ? { choices: k.choices.slice(0, 20) } : {}),
          ...(k.schema ? { schema: boundedJson(k.schema, 2000) } : {}),
        };
      case 'external':
        return { ...base, needs_person: false, ...(k.tool ? { tool: k.tool } : {}) };
      case 'child':
        return { ...base, needs_person: false, child_run_id: k.child_run_id };
      default:
        return base;
    }
  });
}

function boundedJson(v, n) {
  let text;
  try { text = JSON.stringify(v); } catch (_) { return null; }
  return text && text.length > n ? { truncated: true, preview: text.slice(0, n) } : v;
}

/** A child that cannot move on without someone: a request only a person answers, or a pause. */
export function childBlocked(child) {
  return !childDone(child) && (child.status === 'paused'
    || (Array.isArray(child.waiting_for) && child.waiting_for.some((r) => r.needs_person)));
}

/** One line a parent can relay: what the child waits for. */
export function attentionLine(child) {
  const who = child.key || child.run_id;
  if (child.status === 'paused') {
    return `Child ${who} is paused${child.paused_reason ? ` (${JSON.stringify(child.paused_reason)})` : ''}; message-agent resumes it.`;
  }
  const asks = (child.waiting_for || []).filter((r) => r.needs_person).map((r) => (r.kind === 'approval'
    ? `approval of: ${r.summary || 'a change'}`
    : `an answer to: ${r.prompt}${Array.isArray(r.choices) && r.choices.length ? ` (choices: ${r.choices.map((c) => (typeof c === 'string' ? c : (c && (c.label || c.value)) || JSON.stringify(c))).join(', ')})` : ''}`));
  return `Child ${who} is waiting for ${asks.join('; ')}.`;
}

/**
 * The typed spawn specification from a tool call's arguments.
 * Throws `invalid_input` errors a model can repair.
 */
export function normalizeSpawn(args) {
  const a = args && typeof args === 'object' ? args : {};
  const contextMode = str(a.context_mode) || 'none';
  if (!CONTEXT_MODES.includes(contextMode)) throw invalid(`context_mode must be one of ${CONTEXT_MODES.join(', ')}`);
  const tools = a.tools === undefined || a.tools === null ? null : a.tools;
  if (tools !== null && !Array.isArray(tools)) throw invalid('tools must be a list of tool names');
  let context = a.context === undefined ? null : a.context;
  if (context !== null) {
    const text = JSON.stringify(context);
    if (text && text.length > LIMITS.context_chars) throw invalid(`context is larger than ${LIMITS.context_chars} characters; pass only what the child needs`);
  }
  return {
    agent: a.agent_ref === undefined || a.agent_ref === null || a.agent_ref === '' ? null : agentPath(a.agent_ref) || invalidAgent(a.agent_ref),
    key: str(a.key) ? safeName(str(a.key)).slice(0, 40) : null,
    task_id: str(a.task_id) || null,
    objective: normalizeObjective(a.objective),
    context_mode: contextMode,
    recent_turns: int(a.recent_turns, 1, LIMITS.recent_turns, 6),
    context,
    tools: tools === null ? null : tools.map(str).filter(Boolean).slice(0, 50),
    writes: normalizeWrites(a.writes),
    expected_artifacts: normalizeExpected(a.expected_artifacts),
    checks: normalizeChecks(a.checks),
    budget: childBudgets(a.budget),
    budget_request: a.budget && typeof a.budget === 'object' ? a.budget : null,
    independent: a.independent === true,
  };
}

function invalidAgent(ref) {
  throw invalid(`agent_ref must be an installed agent path like /agents/reviewer (got ${JSON.stringify(ref)})`);
}

/**
 * Narrow a child agent's own tools to the grant. A child never gets more
 * than its own agent offers; `grant: null` keeps them all. Delegation tools
 * are removed when the child may not delegate further.
 */
export function narrowTools(agentTools, grant, { mayDelegate }) {
  const want = Array.isArray(grant) ? new Set(grant.map(normToolName)) : null;
  const kept = [];
  const refused = [];
  for (const t of agentTools || []) {
    const isDelegation = DELEGATION_TOOL_PATHS.includes(t.function_path);
    if (isDelegation && !mayDelegate) continue;
    if (want && !want.has(normToolName(t.name)) && !(t.function_path && want.has(normToolName(t.function_path.split('/').pop())))) continue;
    kept.push(t);
  }
  if (want) {
    for (const name of want) {
      if (!kept.some((t) => normToolName(t.name) === name || normToolName(String(t.function_path || '').split('/').pop()) === name)) refused.push(name);
    }
  }
  return { tools: kept, refused };
}

/** Node name of a child's conversation: parent run + the spawn's key. */
export function childChatName(parentRunId, token) {
  return `deleg-${shortRun(parentRunId)}-${safeName(token).slice(0, 24)}`;
}

/** The spawn's stable token (the operation id makes a re-dispatch find its child). */
export function spawnToken(key, opId) {
  return key || digestOf(String(opId || 'spawn'));
}

/** The child's first message: the typed brief it works from. */
export function briefText(spec, { contextBlock, parentAgent, handbackRules = true } = {}) {
  const o = spec.objective;
  const lines = [`You were delegated a bounded task by ${parentAgent || 'another agent'}.`, '', `Objective: ${o.goal}`];
  if (o.deliverable) lines.push(`Deliverable: ${o.deliverable}`);
  if (o.done_when) lines.push(`Done when: ${o.done_when}`);
  if (o.constraints && o.constraints.length) lines.push('Constraints:', ...o.constraints.map((c) => `- ${c}`));
  if (spec.expected_artifacts.length) {
    lines.push('', 'Expected artifacts:', ...spec.expected_artifacts.map((a) => `- ${a.workspace}:${a.path}${a.kind ? ` (${a.kind})` : ''}${a.description ? ` — ${a.description}` : ''}`));
  }
  if (spec.checks.length) {
    lines.push('', 'Acceptance checks (evaluated on the stored result, not on your summary):', ...spec.checks.map(describeCheck));
  }
  if (Array.isArray(spec.writes)) {
    lines.push('', spec.writes.length
      ? `You may write only under: ${spec.writes.map((w) => `${w.workspace}:${w.path}`).join(', ')}.`
      : 'You may not write anything: this is a read-only task.');
  }
  if (contextBlock) lines.push('', contextBlock);
  if (handbackRules) {
    lines.push('', 'Work only on this task. When done, answer with what you inspected, what you changed (paths), how you verified it, and anything unresolved. Say plainly if you could not finish.');
  }
  return lines.join('\n');
}

export function describeCheck(c) {
  switch (c.kind) {
    case 'node_exists': return `- ${c.workspace}:${c.path} exists`;
    case 'artifact_written': return `- ${c.workspace}:${c.path} is written by you`;
    case 'property_equals': return `- ${c.workspace}:${c.path} has ${c.property} = ${JSON.stringify(c.value)}`;
    case 'outcome_is': return `- the run finishes ${c.value}`;
    default: return `- ${c.kind}`;
  }
}

/** Whether a child counts as finished for a wait: its run is terminal, or it handed back. */
export function childDone(child) {
  return TERMINAL_STATUSES.has(child.status) || !!child.handed_back;
}

/** Whether a wait over `children` is satisfied in `mode` (`all` | `any`). */
export function waitSatisfied(mode, children) {
  if (!children.length) return true;
  return mode === 'any' ? children.some(childDone) : children.every(childDone);
}

/**
 * Evaluate expected artifacts and checks against what exists and what the
 * child wrote. `exists(ws, path) -> node|null` is looked up by the caller
 * beforehand into `nodes` (a Map of `ws:path` → node) so this stays pure.
 */
export function acceptance(spec, { outcome, written, nodes }) {
  const has = (ws, p) => nodes.has(`${ws}:${p}`) && nodes.get(`${ws}:${p}`);
  const wrote = (ws, p) => written.some((w) => w && w.workspace === ws && w.path === p);
  const expected = (spec.expected_artifacts || []).map((a) => ({ ...a, found: !!has(a.workspace, a.path) }));
  const checks = (spec.checks || []).map((c) => {
    switch (c.kind) {
      case 'node_exists': return { ...c, ok: !!has(c.workspace, c.path) };
      case 'artifact_written': return { ...c, ok: wrote(c.workspace, c.path) };
      case 'property_equals': {
        const n = has(c.workspace, c.path);
        const actual = n && n.properties ? n.properties[c.property] : undefined;
        return { ...c, ok: n ? JSON.stringify(actual) === JSON.stringify(c.value) : false, actual: actual === undefined ? null : actual };
      }
      case 'outcome_is': return { ...c, ok: outcome === c.value, actual: outcome };
      default: return { ...c, ok: false };
    }
  });
  const accepted = outcome === 'succeeded' && expected.every((e) => e.found) && checks.every((c) => c.ok);
  return { expected, checks, accepted };
}

/** Locators every check/expectation reads (the caller loads them). */
export function acceptanceReads(spec) {
  const out = new Map();
  for (const a of spec.expected_artifacts || []) out.set(`${a.workspace}:${a.path}`, a);
  for (const c of spec.checks || []) if (c.workspace && c.kind !== 'artifact_written') out.set(`${c.workspace}:${c.path}`, c);
  return [...out.values()].map((x) => ({ workspace: x.workspace, path: x.path }));
}

/** The spawn key of a spec: the model's key, the task binding, or the operation's digest. */
export function spawnKeyOf(spec, opId) {
  if (spec.key) return spec.key;
  if (spec.task_id) return taskKey(spec.task_id);
  return spawnToken(null, opId);
}

/** The spawn key a plan task binds a child to (`delegate-task`). */
export function taskKey(taskId) {
  return `task-${safeName(String(taskId)).slice(0, 30)}`;
}

/**
 * The typed objective core stores on the child (`ChildObjective`). Expected
 * artifacts and checks travel as data core carries but does not judge
 * (`required: false`): ai-tools evaluates them on STORED state when the
 * parent reads the child (`acceptance`), because only stored state can prove
 * a node exists or holds a value.
 */
export function coreObjective(spec, { brief, allowedTools, contextSelection }) {
  const o = spec.objective;
  return {
    title: o.goal.slice(0, 200),
    instructions: brief,
    context: contextSelection || { mode: 'none' },
    allowed_tools: allowedTools,
    ...(Array.isArray(spec.writes) ? { allowed_writes: spec.writes } : {}),
    expected_artifacts: spec.expected_artifacts.map((a) => ({
      kind: a.kind || 'node',
      locator: { workspace: a.workspace, path: a.path },
      ...(a.description ? { description: a.description } : {}),
      required: false,
    })),
    acceptance_checks: spec.checks.map((c, i) => ({
      id: `check-${i + 1}`,
      description: describeCheck(c).replace(/^- /, ''),
      check: c,
      required: false,
    })),
    hand_back: {
      description: [o.deliverable, o.done_when && `Done when: ${o.done_when}`].filter(Boolean).join(' ') || null,
    },
  };
}

/** The expected artifacts and checks back from a stored objective (for `acceptance`). */
export function specOfObjective(objective) {
  const obj = objective && typeof objective === 'object' ? objective : {};
  const expected = (Array.isArray(obj.expected_artifacts) ? obj.expected_artifacts : [])
    .map((a) => (a && a.locator && a.locator.workspace && a.locator.path
      ? { workspace: a.locator.workspace, path: a.locator.path, kind: a.kind, ...(a.description ? { description: a.description } : {}) }
      : null))
    .filter(Boolean);
  const checks = (Array.isArray(obj.acceptance_checks) ? obj.acceptance_checks : [])
    .map((c) => (c && c.check && CHECK_KINDS.has(c.check.kind) ? c.check : null))
    .filter(Boolean);
  return { expected_artifacts: expected, checks };
}
