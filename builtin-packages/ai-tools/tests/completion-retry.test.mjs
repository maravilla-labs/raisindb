/**
 * ONE MALFORMED TOOL CALL MUST NOT END THE TURN (baseline gap 11, 2026-09-21).
 *
 * Measured: Studio Builder (groq / gpt-oss-120b) emitted an execute-function
 * call whose arguments were not JSON, the provider refused the completion, and
 * the turn ended on "Error: Backend error: … Failed to parse tool call arguments
 * as JSON" — no retry, and nothing said about where the run stood.
 *
 * Unit tests for agent-shared/completion-retry.js (the model-turn function's
 * retry of one malformed call).
 *
 * Run: node --test builtin-packages/ai-tools/tests/completion-retry.test.mjs
 */
import assert from 'node:assert/strict';
import test from 'node:test';

import {
  completeWithToolCallRetry,
  isMalformedToolCallError,
  failedToolName,
  MALFORMED_TOOL_CALL_CODE,
} from '../content/functions/lib/raisin/ai/agent-shared/completion-retry.js';

/** The exact provider refusal from the round 1 B conversation (shortened). */
const GROQ_REFUSAL =
  'Backend error: AI fallback failed: API request failed: Failed to parse tool call arguments as JSON\n' +
  'Failed generation: {"name": "execute-function", "arguments": {"path":"/lib/studio/generated/homepage-title-validator","cases":[{"name":"missing prefix only","input":{"title":"My Site"},"expect":{"reason":"Add';

// ── the helper ──────────────────────────────────────────────────────────────

test('the round 1 B refusal is recognised, and names its tool', () => {
  assert.equal(isMalformedToolCallError(new Error(GROQ_REFUSAL)), true);
  assert.equal(failedToolName(new Error(GROQ_REFUSAL)), 'execute-function');
  assert.equal(isMalformedToolCallError(new Error('rate limit exceeded')), false);
  assert.equal(isMalformedToolCallError(new Error('connection reset')), false);
});

test('one malformed call is retried once, with a correction the model can act on', async () => {
  const seen = [];
  const out = await completeWithToolCallRetry(async (messages) => {
    seen.push(messages);
    if (seen.length === 1) throw new Error(GROQ_REFUSAL);
    return { content: 'ok' };
  }, [{ role: 'user', content: 'go' }]);
  assert.equal(out.retried, true);
  assert.equal(seen.length, 2);
  const correction = seen[1][seen[1].length - 1];
  assert.equal(correction.role, 'system');
  assert.match(correction.content, /execute-function/);
  assert.match(correction.content, /valid JSON/);
  assert.equal(seen[1].length, seen[0].length + 1, 'history is kept; the correction is appended');
});

test('a second malformed call is thrown as malformed_tool_call, after exactly two attempts', async () => {
  let calls = 0;
  await assert.rejects(
    () => completeWithToolCallRetry(async () => { calls++; throw new Error(GROQ_REFUSAL); }, []),
    (err) => err.code === MALFORMED_TOOL_CALL_CODE && err.attempts === 2 && err.tool === 'execute-function',
  );
  assert.equal(calls, 2);
});

test('any other error is not retried', async () => {
  let calls = 0;
  await assert.rejects(
    () => completeWithToolCallRetry(async () => { calls++; throw new Error('rate limit exceeded'); }, []),
    /rate limit/,
  );
  assert.equal(calls, 1);
});
