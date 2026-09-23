/**
 * node-list — `raisin.nodeDev.list` as an agent tool.
 *
 * Everything that matters happens in core (`raisin.node_dev.*`): the roots
 * bound every path, an agent run's own grant (read from the run record,
 * never from these arguments) narrows them, row-level security applies as
 * the caller, and inside a run (`__raisin_context`) the answer is a
 * `raisin.tool-result/1` envelope whose idempotency key is the run's
 * operation id. This file only forwards.
 */
export async function handler(input) {
  return raisin.nodeDev.list(input || {});
}
