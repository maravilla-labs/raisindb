/**
 * `raisin.tool-result/1` — the one result shape every ai-tools tool returns
 * when an AgentRun invokes it.
 *
 * A tool is invoked by a run when its args carry `__raisin_context.run_id`
 * and `.operation_id` (core injects both). Outside a run (a direct caller, a
 * flow step) the tool answers with its plain result — the envelope never
 * leaks to a caller that did not ask for it.
 *
 * The envelope:
 *   envelope, operation_id, status (succeeded|waiting|retryable|blocked|failed),
 *   writes[], artifact_refs[], evidence[], diagnostics[],
 *   suggested_next_actions[], retry_policy, payload (the tool's own result)
 *
 * The operation id is the IDEMPOTENCY KEY of a mutating tool: a re-dispatched
 * operation reuses it, and a tool must answer a repeat with the first result
 * instead of writing twice (see `idempotencyKey`).
 */

import { TOOL_RESULT_ENVELOPE, digestOf } from './run-names.js';

/** Error classes, shared with the model-turn function. */
export const ERROR_CLASSES = Object.freeze({
  INVALID_INPUT: 'invalid_input',
  NOT_FOUND: 'not_found',
  PERMISSION_DENIED: 'permission_denied',
  CONFLICT: 'conflict',
  TRANSIENT: 'transient',
  TIMEOUT: 'timeout',
  RATE_LIMITED: 'rate_limited',
  UNSUPPORTED: 'unsupported',
  TOOL_ERROR: 'tool_error',
});

const PATTERNS = [
  [ERROR_CLASSES.RATE_LIMITED, /rate.?limit|too many requests|\b429\b|quota/i],
  [ERROR_CLASSES.TIMEOUT, /timed? ?out|timeout|deadline exceeded/i],
  [ERROR_CLASSES.TRANSIENT, /\b50[0234]\b|overloaded|temporarily|unavailable|econnreset|socket hang up|network|connection (reset|refused|closed)/i],
  [ERROR_CLASSES.PERMISSION_DENIED, /permission denied|forbidden|not allowed|unauthori[sz]ed|\b403\b|access denied/i],
  [ERROR_CLASSES.NOT_FOUND, /not found|does not exist|no such|\b404\b/i],
  [ERROR_CLASSES.CONFLICT, /already exists|conflict|stale|revision mismatch|\b409\b/i],
  [ERROR_CLASSES.UNSUPPORTED, /not supported|unsupported|not implemented/i],
  [ERROR_CLASSES.INVALID_INPUT, /required|invalid|must be|expected|missing|malformed|validation/i],
];

/** The class of an error (or its message). */
export function classifyError(err) {
  if (err && typeof err === 'object' && typeof err.error_class === 'string') return err.error_class;
  const text = String((err && err.message) || err || '');
  for (const [cls, re] of PATTERNS) if (re.test(text)) return cls;
  return ERROR_CLASSES.TOOL_ERROR;
}

/** Envelope status for an error class. */
export function statusForClass(cls) {
  switch (cls) {
    case ERROR_CLASSES.TRANSIENT:
    case ERROR_CLASSES.TIMEOUT:
    case ERROR_CLASSES.RATE_LIMITED:
      return 'retryable';
    case ERROR_CLASSES.PERMISSION_DENIED:
    case ERROR_CLASSES.UNSUPPORTED:
      return 'blocked';
    default:
      return 'failed';
  }
}

/** Retry policy for an error class. */
export function retryPolicyFor(cls) {
  const retryable = statusForClass(cls) === 'retryable';
  return retryable
    ? { retryable: true, max_attempts: 3, backoff_ms: cls === ERROR_CLASSES.RATE_LIMITED ? 5000 : 1000, reason: cls }
    : { retryable: false, max_attempts: 1, backoff_ms: 0, reason: cls };
}

/** The run identity a tool was invoked with, or null outside a run. */
export function runContextOf(input) {
  const ctx = input && typeof input === 'object' ? input.__raisin_context : null;
  if (!ctx || typeof ctx !== 'object') return null;
  if (typeof ctx.run_id !== 'string' || typeof ctx.operation_id !== 'string') return null;
  return ctx;
}

/**
 * The idempotency key of this invocation: the run's operation id, else an
 * explicit `idempotency_key` argument, else null (no dedupe possible).
 */
export function idempotencyKey(input) {
  const ctx = runContextOf(input);
  if (ctx) return ctx.operation_id;
  const key = input && typeof input.idempotency_key === 'string' ? input.idempotency_key.trim() : '';
  return key || null;
}

/** A stable short token for names derived from an idempotency key. */
export function keyToken(key) {
  return digestOf(String(key));
}

/** A node locator. */
export function locator(workspace, path, nodeId = null) {
  return { workspace, path, node_id: nodeId || null };
}

/** A diagnostic entry. */
export function diagnostic(code, message, { severity = 'error', cls = null, fix = null, path = null } = {}) {
  return { code, severity, message: String(message), class: cls, fix, path };
}

/**
 * Build an envelope. A `succeeded` envelope with writes gets exactly one
 * primary artifact ref (the contract's rule): the first ref marked primary,
 * else the first write promoted.
 */
export function buildEnvelope({
  operationId,
  status = 'succeeded',
  payload = null,
  writes = [],
  artifactRefs = [],
  evidence = [],
  diagnostics = [],
  suggestedNextActions = [],
  retryPolicy = null,
  resumeKey = null,
  reads = [],
}) {
  const refs = Array.isArray(artifactRefs) ? artifactRefs.slice() : [];
  if (status === 'succeeded' && writes.length > 0) {
    const primaries = refs.filter((r) => r.role === 'primary');
    if (primaries.length === 0) {
      refs.unshift({ kind: 'node', locator: writes[0].locator, role: 'primary', logical_key: null });
    } else if (primaries.length > 1) {
      let seen = false;
      for (const r of refs) {
        if (r.role !== 'primary') continue;
        if (seen) r.role = 'supporting';
        seen = true;
      }
    }
  }
  const env = {
    envelope: TOOL_RESULT_ENVELOPE,
    operation_id: operationId,
    status,
    reads,
    writes,
    artifact_refs: refs,
    evidence,
    diagnostics,
    suggested_next_actions: suggestedNextActions,
    retry_policy: retryPolicy || { retryable: status === 'retryable', max_attempts: 1, backoff_ms: 0 },
    payload,
  };
  if (status === 'waiting') env.resume_key = resumeKey || operationId;
  return env;
}

/** The envelope of a thrown error. */
export function errorEnvelope(operationId, err, extra = {}) {
  const cls = classifyError(err);
  const message = String((err && err.message) || err || 'tool failed');
  return buildEnvelope({
    operationId,
    status: statusForClass(cls),
    payload: { error: message, error_class: cls },
    diagnostics: [diagnostic(extra.code || cls, message, {
      cls: statusForClass(cls) === 'retryable' ? 'transient' : (cls === ERROR_CLASSES.INVALID_INPUT ? 'repairable' : 'blocking'),
      fix: extra.fix || null,
    })],
    suggestedNextActions: extra.next || [],
    retryPolicy: retryPolicyFor(cls),
  });
}

/**
 * Run a tool body under the envelope when (and only when) a run invoked it.
 *
 * Usage, as the FIRST line of a tool's handler:
 *   const enveloped = await runEnvelope(input, META, handler); if (enveloped) return enveloped;
 *
 * `meta`: { tool, mutating, writes?(result, input) -> [{locator, action}],
 *           artifacts?(result, input) -> [artifact_ref], evidence?(result, input),
 *           next?(result, input) -> [{action, args?, reason}] }
 * The body receives `{...input, __envelope_inner: true}` and returns its
 * ordinary result; `success: false` or an `error` field reads as failed.
 */
export async function runEnvelope(input, meta, body) {
  const ctx = runContextOf(input);
  if (!ctx || (input && input.__envelope_inner)) return null;
  const opId = ctx.operation_id;
  let result;
  try {
    result = await body({ ...input, __envelope_inner: true });
  } catch (err) {
    return errorEnvelope(opId, err);
  }
  const failed = result && typeof result === 'object'
    && (result.success === false || (typeof result.error === 'string' && result.error && result.success !== true));
  if (failed) {
    const message = result.error || result.message || `${meta.tool} did not succeed`;
    const env = errorEnvelope(opId, message);
    env.payload = result;
    return env;
  }
  const writes = safeCall(meta.writes, result, input) || [];
  return buildEnvelope({
    operationId: opId,
    status: 'succeeded',
    payload: result,
    writes,
    artifactRefs: safeCall(meta.artifacts, result, input) || [],
    evidence: safeCall(meta.evidence, result, input) || [],
    suggestedNextActions: safeCall(meta.next, result, input) || [],
    retryPolicy: { retryable: false, max_attempts: 1, backoff_ms: 0, reason: meta.mutating ? 'idempotent_by_operation_id' : 'read_only' },
  });
}

function safeCall(fn, result, input) {
  if (typeof fn !== 'function') return null;
  try {
    return fn(result, input);
  } catch (_) {
    return null;
  }
}

/** `writes` helper: one updated/created node. */
export function writeOf(workspace, path, action = 'updated', nodeId = null) {
  if (!workspace || !path) return [];
  return [{ locator: locator(workspace, path, nodeId), action }];
}
