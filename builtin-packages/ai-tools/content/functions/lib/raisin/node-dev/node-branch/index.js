/**
 * node-branch — fork / diff / merge / discard a draft branch, as an agent
 * tool. Merges report conflicts instead of guessing; core refuses to discard
 * a protected or root branch, or one the caller did not create.
 */
const ACTIONS = {
  fork: (i) => raisin.nodeDev.forkBranch(i),
  diff: (i) => raisin.nodeDev.diffBranch(i),
  merge: (i) => raisin.nodeDev.mergeBranch(i),
  discard: (i) => raisin.nodeDev.discardBranch(i),
};

export async function handler(input) {
  const i = input || {};
  const run = ACTIONS[i.action];
  if (!run) {
    throw new Error(`node-branch: action must be one of ${Object.keys(ACTIONS).join(', ')}`);
  }
  const { action, ...rest } = i;
  return run(rest);
}
