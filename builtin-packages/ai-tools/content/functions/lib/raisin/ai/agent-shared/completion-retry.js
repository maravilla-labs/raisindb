/**
 * ONE MALFORMED TOOL CALL MUST NOT END THE TURN.
 *
 * Measured 2026-09-21 (Studio Builder, round 1 B, groq / gpt-oss-120b): the
 * model emitted an `execute-function` call whose arguments were not valid JSON.
 * The provider refused the whole completion ("Failed to parse tool call
 * arguments as JSON / Failed generation: …"), `raisin.ai.completion` threw, and
 * the handler wrote "Error: Backend error: …" and stopped. One bad token in a
 * long argument ended a build that was otherwise progressing, with no retry and
 * no statement of where the run had got to.
 *
 * So: ONE retry, with a short correction appended for the model. If the retry
 * fails the same way the turn ends — but with an honest terminal status, the
 * finalize gate's own statement for agents that declare the policy, never the
 * raw provider error alone and never silently.
 *
 * The same applies to the other shape of the fault: a completion that RETURNS
 * but whose only tool calls were dropped as malformed by `normalizeToolCalls`.
 */

import { finalizePolicyOf, FINALIZE_POLICY_VERIFIED } from './finalize.js';
import { gateTerminalContentWithRunWrites } from './run-evidence.js';

/** Provider refusals that mean "the model's tool call was not parseable". */
const MALFORMED_PATTERNS = [
  /failed to parse tool call arguments/i,
  /tool_use_failed/i,
  /tool call validation failed/i,
  /invalid tool arguments json/i,
  /failed_generation/i,
];

export const MALFORMED_TOOL_CALL_CODE = 'malformed_tool_call';

export function isMalformedToolCallError(err) {
  if (!err) return false;
  if (err.code === MALFORMED_TOOL_CALL_CODE) return true;
  const text = String(err.message || err);
  return MALFORMED_PATTERNS.some((re) => re.test(text));
}

/** The tool the model was trying to call, when the provider quoted it. */
export function failedToolName(errOrText) {
  const text = String((errOrText && errOrText.message) || errOrText || '');
  const quoted = text.match(/"name"\s*:\s*"([\w.:-]+)"/);
  if (quoted) return quoted[1];
  const fn = text.match(/<function=([\w.:-]+)/);
  return fn ? fn[1] : null;
}

/** The short correction the model sees on its one retry. */
export function malformedToolCallCorrection(errOrText) {
  const tool = failedToolName(errOrText);
  return {
    role: 'system',
    content:
      `Your last tool call${tool ? ` to ${tool}` : ''} was rejected because its arguments were not valid JSON. ` +
      'Call the tool again with arguments that are ONE valid JSON object: double-quoted keys and strings, ' +
      'no trailing commas, no comments, every bracket and quote closed, special characters escaped. ' +
      'If the arguments were long, send a shorter version (fewer cases, shorter strings).',
  };
}

/**
 * Call the model; on a malformed-tool-call refusal, call it ONCE more with the
 * correction appended. A second malformed refusal is re-thrown with
 * `code: 'malformed_tool_call'` and `attempts: 2`; any other error is re-thrown
 * as it came. `complete(messages)` performs the actual completion.
 */
export async function completeWithToolCallRetry(complete, messages, onRetry) {
  try {
    return { raw: await complete(messages), retried: false };
  } catch (err) {
    if (!isMalformedToolCallError(err)) throw err;
    if (typeof onRetry === 'function') onRetry(err);
    const corrected = [...messages, malformedToolCallCorrection(err)];
    try {
      return { raw: await complete(corrected), retried: true, first_error: String(err.message || err) };
    } catch (err2) {
      if (!isMalformedToolCallError(err2)) throw err2;
      const out = new Error(String(err2.message || err2));
      out.code = MALFORMED_TOOL_CALL_CODE;
      out.attempts = 2;
      out.tool = failedToolName(err2) || failedToolName(err);
      out.cause = err2;
      throw out;
    }
  }
}

/**
 * What the turn says when the retry failed too. For an agent that declares the
 * finalize policy the gate's statement is appended — the run's state comes
 * from persisted tasks and evidence, not from the model.
 */
export async function malformedToolCallStopText(workspace, chatPath, agentProps, err) {
  const tool = (err && err.tool) || failedToolName(err);
  let text =
    `This turn stopped: the model sent a tool call${tool ? ` to ${tool}` : ''} whose arguments were not valid JSON, ` +
    'was asked once to correct it, and sent an invalid one again. That call did not run. ' +
    'Send a message to continue from here.';
  if (finalizePolicyOf(agentProps) === FINALIZE_POLICY_VERIFIED) {
    // The same statement the terminal gate would make — including writes this
    // run made that no record covers, and records a later write made stale.
    const { statement } = await gateTerminalContentWithRunWrites(workspace, chatPath, agentProps, '');
    text += `\n\n${statement || 'Status (server-verified): nothing in this run has been verified.'}`;
  }
  return text;
}
