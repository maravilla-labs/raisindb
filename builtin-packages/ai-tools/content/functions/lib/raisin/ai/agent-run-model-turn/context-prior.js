/**
 * Opt-in shortening of EARLIER runs' tool results (agent property
 * `prior_run_tool_result_chars`).
 *
 * A chat holds several runs (every follow-up message starts a new one) and the
 * context is rebuilt from the whole chat, so a new run inherits the earlier
 * runs' tool results in full. Calls and assistant text stay; only the result
 * bodies of runs OTHER than the executing one are cut to their first N
 * characters. Pure: no `raisin.*`.
 *
 * Attribution: with `tagRuns`, buildHistoryFromChat marks each assistant entry
 * and its tool results with `__run_id` (the message's `run_id` property, else
 * the `run-<short>-op-<n>` node name). An entry whose id does not match the
 * executing run (or that has no id: a pre-run chat message) is "earlier".
 */

import { shortRun } from '../agent-shared/run-names.js';

function sameRun(tag, runId) {
  if (!tag) return false;
  return tag === runId || tag === shortRun(runId) || shortRun(tag) === shortRun(runId);
}

/** Cut earlier runs' tool results to `maxChars`. Pairing and order are untouched. */
export function shortenPriorRunResults(history, runId, maxChars) {
  if (!(maxChars > 0)) return { history, shortened: 0 };
  let shortened = 0;
  const out = history.map((e) => {
    if (e.role !== 'tool' || typeof e.content !== 'string') return e;
    if (sameRun(e.__run_id, runId) || e.content.length <= maxChars) return e;
    shortened += 1;
    return {
      ...e,
      content: `${e.content.slice(0, maxChars)}\n[earlier run: ${maxChars} of ${e.content.length} characters kept; call the tool again if you need the rest]`,
    };
  });
  return { history: out, shortened };
}

/** Remove the `__run_id` markers before the messages reach a provider. */
export function stripRunTags(messages) {
  return messages.map((m) => {
    if (!m || !('__run_id' in m)) return m;
    const { __run_id, ...rest } = m;
    return rest;
  });
}
