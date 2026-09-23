/**
 * agent-run-reducer — the generic agent loop of every ai-tools conversation
 * run, as an ordinary RaisinDB function speaking `raisin.agent-run.reducer/1`.
 *
 * Core calls it inline under the deterministic execution policy (no host
 * calls, frozen clock, fixed entropy). See reduce.js for the loop, turn.js for
 * the tool gate and loop detection, plan.js for the plan as run state.
 */

import { reduce } from './reduce.js';

export async function handler(request) {
  return reduce(request);
}
