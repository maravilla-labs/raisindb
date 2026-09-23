/**
 * Names, ids and digests shared by every AgentRun participant in ai-tools.
 *
 * PURE: no `raisin.*`, no clock, no randomness. The generic reducer runs under
 * the deterministic execution policy and imports this module, so anything
 * added here must stay pure.
 */

/** The generic agent-loop reducer every conversation run uses by default. */
export const DEFAULT_RUN_REDUCER = '/lib/raisin/ai/agent-run-reducer';
/** The model-turn function core calls for `request_model_turn`. */
export const DEFAULT_MODEL_TURN_FUNCTION = '/lib/raisin/ai/agent-run-model-turn';
/** The transcript/plan projection operation the reducer issues itself. */
export const PROJECT_FUNCTION = '/lib/raisin/ai/agent-run-project';
/** The reducer contract spoken by ai-tools. */
export const REDUCER_CONTRACT = 'raisin.agent-run.reducer/1';
/** The tool-result envelope id. */
export const TOOL_RESULT_ENVELOPE = 'raisin.tool-result/1';

/** `{run_id}/op/{n}` → n (0 when the id has no op suffix). */
export function opIndex(operationId) {
  const m = /\/op\/(\d+)$/.exec(String(operationId || ''));
  return m ? Number(m[1]) : 0;
}

/** First 8 id characters, enough to keep two runs of one chat apart. */
export function shortRun(runId) {
  return String(runId || 'run').replace(/[^A-Za-z0-9]/g, '').slice(0, 8) || 'run';
}

/**
 * The assistant message a model-turn operation writes. Deterministic, so a
 * replayed operation finds its own message instead of writing a second one.
 */
export function turnMessageName(runId, operationId) {
  return `run-${shortRun(runId)}-op-${opIndex(operationId)}`;
}

/** Path of that message inside the conversation. */
export function turnMessagePath(chatPath, runId, operationId) {
  return `${chatPath}/${turnMessageName(runId, operationId)}`;
}

/** A node-name-safe spelling of a tool call id. */
export function safeName(value) {
  return String(value || '').replace(/[^A-Za-z0-9_-]/g, '_').slice(0, 96) || 'x';
}

/** JSON with object keys sorted at every depth — the only form ever digested. */
export function canonicalJson(value) {
  if (value === null || typeof value !== 'object') {
    return JSON.stringify(value === undefined ? null : value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  const keys = Object.keys(value).filter((k) => value[k] !== undefined).sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalJson(value[k])}`).join(',')}}`;
}

/** 53-bit string hash (cyrb53), hex. Stable across runtimes. */
export function hash53(text) {
  const str = String(text);
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < str.length; i++) {
    const ch = str.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  const n = 4294967296 * (2097151 & h2) + (h1 >>> 0);
  return n.toString(16).padStart(14, '0');
}

/** The digest algorithm name carried beside every digest this package makes. */
export const DIGEST_ALG = 'cyrb53-canonical-json';

/** Digest of a value's canonical JSON. */
export function digestOf(value) {
  return hash53(canonicalJson(value));
}

/** Tool names compare without regard to `-`/`_` or case (`create-plan` = `create_plan`). */
export function normToolName(name) {
  return String(name || '').trim().replace(/-/g, '_').toLowerCase();
}

/** `"ws:/path"` | `"/path"` | `{raisin:path, raisin:workspace}` → `{workspace, path}`. */
export function splitAgentRef(ref, defaultWorkspace = 'functions') {
  if (ref && typeof ref === 'object') {
    return {
      workspace: ref['raisin:workspace'] || ref.workspace || defaultWorkspace,
      path: ref['raisin:path'] || ref.path || null,
    };
  }
  const text = String(ref || '');
  const idx = text.indexOf(':/');
  if (idx > 0) return { workspace: text.slice(0, idx), path: text.slice(idx + 1) };
  return { workspace: defaultWorkspace, path: text || null };
}

/** Which plan operation a planning tool performs, from its function path or name. */
export function planOpOf(pathOrName) {
  const tail = String(pathOrName || '').split('/').pop().replace(/_/g, '-').toLowerCase();
  return {
    'create-plan': 'plan.create',
    'add-task': 'plan.add_task',
    'update-task': 'plan.update_task',
    'get-plan-status': 'plan.status',
  }[tail] || null;
}
