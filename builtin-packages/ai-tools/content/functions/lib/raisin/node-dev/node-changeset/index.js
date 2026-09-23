/**
 * node-changeset — get / commit / discard / list a changeset, as an agent
 * tool. See node-apply for what core guarantees; this file only routes.
 */
const ACTIONS = {
  get: (i) => raisin.nodeDev.getChangeset(i),
  commit: (i) => raisin.nodeDev.commit(i),
  discard: (i) => raisin.nodeDev.discard(i),
  list: (i) => raisin.nodeDev.listChangesets(i),
};

export async function handler(input) {
  const i = input || {};
  const run = ACTIONS[i.action];
  if (!run) {
    throw new Error(`node-changeset: action must be one of ${Object.keys(ACTIONS).join(', ')}`);
  }
  const { action, ...rest } = i;
  return run(rest);
}
