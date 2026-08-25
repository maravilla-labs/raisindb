/**
 * Render the lowered execution plan as text (flow explain).
 */

import { lowerFlow, type LoweredFlow, type PlanNode } from './lower.js';
import type { FlowSource } from './types.js';

const KIND_LABELS: Record<string, string> = {
  start: 'start',
  end: 'end',
  function: 'function',
  ai_agent: 'ai agent (single-shot)',
  ai_container: 'ai container (tool loop)',
  chat: 'chat session',
  human_task: 'human task',
  decision: 'decision',
  sequence: 'sequence',
  parallel: 'parallel',
  loop: 'loop',
};

/**
 * Walk the plan from `start`, collecting display order: depth-first along
 * the normal flow (next, then decision branches, then error paths), with
 * `end` always printed last.
 */
function displayOrder(plan: LoweredFlow): string[] {
  const order: string[] = [];
  const visited = new Set<string>(['end']);
  const stack = ['start'];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (visited.has(id)) continue;
    visited.add(id);
    order.push(id);
    const node = plan.nodes.get(id);
    if (!node) continue;
    const successors = [
      node.body,
      node.next,
      node.yes,
      node.no,
      ...(node.branches ?? []).map((b) => b.entry),
      ...(node.tools ?? []),
      node.errorEdge,
      node.timeoutEdge,
    ].filter((n): n is string => !!n && plan.nodes.has(n) && !visited.has(n));
    // Reverse so the first successor (normal flow) is processed first
    for (let i = successors.length - 1; i >= 0; i--) stack.push(successors[i]);
  }
  // Append anything unreachable (e.g. unrouted OR children)
  for (const id of plan.nodes.keys()) {
    if (!visited.has(id) && id !== 'end') order.push(id);
  }
  if (plan.nodes.has('end')) order.push('end');
  return order;
}

function renderNode(node: PlanNode, lines: string[]): void {
  const kind = KIND_LABELS[node.kind] ?? node.kind;
  const label = node.label ? `  "${node.label}"` : '';
  if (node.kind === 'start') {
    lines.push(`  start → ${node.next}`);
    return;
  }
  if (node.kind === 'end') {
    lines.push('  end');
    return;
  }
  lines.push(`  ${node.id}  [${kind}]${label}`);
  for (const d of node.detail) lines.push(`      ${d}`);
  if (node.kind === 'decision') {
    lines.push(`      if ${node.condition || '(empty condition)'}`);
    lines.push(`        yes → ${node.yes}    no → ${node.no}`);
  }
  if (node.body) lines.push(`      body → ${node.body} (loops back here per item)`);
  for (const b of node.branches ?? []) {
    lines.push(`      branch "${b.id}" → ${b.entry}`);
  }
  if ((node.tools ?? []).length > 0) {
    lines.push(`      explicit tool steps: ${node.tools!.join(', ')}`);
  }
  if (node.errorEdge) lines.push(`      on error → ${node.errorEdge}`);
  if (node.timeoutEdge) lines.push(`      on timeout → ${node.timeoutEdge}`);
  if (node.next && node.kind !== 'start') lines.push(`      next → ${node.next}`);
}

/** Render an execution-plan summary for a designer flow source. */
export function explainFlow(source: FlowSource): string {
  const lines: string[] = [];
  lines.push(`Flow: ${source.name}`);
  lines.push(`File: ${source.file}`);
  if (source.note) lines.push(`Note: ${source.note}`);

  if (source.format !== 'designer' || !source.definition) {
    lines.push('');
    lines.push('Definition is in the runtime format (explicit start/end and next_node chaining)');
    lines.push('— it is executed as-is; flow explain only lowers the designer format.');
    return lines.join('\n');
  }

  const def = source.definition;
  const plan = lowerFlow(def);
  const designerCount = countNodes(def.nodes ?? []);
  lines.push(
    `Format: designer v${def.version ?? 1}, error_strategy=${def.error_strategy ?? 'fail_fast'}`
  );
  lines.push(`Lowered: ${designerCount} designer node(s) → ${plan.nodes.size} runtime node(s)`);
  lines.push('');
  lines.push('Execution plan:');
  for (const id of displayOrder(plan)) {
    const node = plan.nodes.get(id);
    if (node) renderNode(node, lines);
  }

  const compensations = [...plan.nodes.values()].filter((n) =>
    n.detail.some((d) => d.startsWith('compensation:'))
  );
  if (compensations.length > 0) {
    lines.push('');
    lines.push('Saga compensation (runs in LIFO order on unrecoverable failure):');
    for (const n of compensations) {
      const comp = n.detail.find((d) => d.startsWith('compensation:'))!;
      lines.push(`  ${n.id}: ${comp.replace('compensation: ', '')}`);
    }
  }
  return lines.join('\n');
}

function countNodes(nodes: Array<{ children?: unknown[] }>): number {
  let n = 0;
  for (const node of nodes) {
    n += 1;
    if (Array.isArray(node.children)) n += countNodes(node.children as Array<{ children?: unknown[] }>);
  }
  return n;
}
