/**
 * Designer-format lowering, structurally faithful to
 * crates/raisin-flow-runtime/src/types/designer_format/conversion.rs:
 *
 * - sibling nodes chain to the next sibling, the last to the successor
 * - AND containers become a pass-through into the child chain
 * - OR containers become a decision cascade (container id = first rule,
 *   extra rules become <id>__rule<k>); no match falls through to successor
 * - parallel containers fork each child into its own branch flow
 * - ai_sequence containers become a single AI container node (children are
 *   tool steps)
 * - implicit start/end nodes are injected
 */

import {
  determineStepKind,
  isContainer,
  isStep,
  normalizeReference,
  type DesignerFlowDefinition,
  type DesignerNode,
} from './types.js';

export interface PlanBranch {
  id: string;
  entry: string;
}

export interface PlanNode {
  id: string;
  /** start | end | function | ai_agent | ai_container | chat | human_task | wait | sub_flow | decision | sequence | parallel | loop */
  kind: string;
  label?: string;
  /** Extra human-readable details (function ref, assignee, retry, ...). */
  detail: string[];
  next?: string;
  /** Decision routing */
  condition?: string;
  yes?: string;
  no?: string;
  /** Parallel branches */
  branches?: PlanBranch[];
  /** ai_sequence explicit tool steps */
  tools?: string[];
  /** Loop body entry (the body chain ends back at the loop node) */
  body?: string;
  errorEdge?: string;
  timeoutEdge?: string;
}

export interface LoweredFlow {
  entry: string;
  nodes: Map<string, PlanNode>;
}

function refPath(raw: unknown): string | undefined {
  const ref = normalizeReference(raw as never);
  if (!ref) return typeof raw === 'string' ? raw : undefined;
  return ref.path ?? ref.ref;
}

function lowerStep(node: DesignerNode, successor: string, out: Map<string, PlanNode>): string {
  const id = node.id ?? '<missing-id>';
  const props = node.properties ?? {};
  const kind = determineStepKind(props);
  const detail: string[] = [];

  const fn = refPath(props.function_ref);
  if (fn) detail.push(`function: ${fn}`);
  const agent = refPath(props.agent_ref);
  if (agent) detail.push(`agent: ${agent}`);
  if (kind === 'chat') {
    const chatAgent = refPath(props.chat_config?.agent_ref ?? undefined);
    detail.push(`chat agent: ${chatAgent ?? '(resolved from context)'}`);
  }
  if (kind === 'human_task') {
    detail.push(`task: ${props.task_type ?? '?'} → ${props.assignee ?? '(no assignee)'}`);
    if (props.escalation_assignee) detail.push(`escalates to: ${props.escalation_assignee}`);
  }
  if (kind === 'wait') {
    const waitType = typeof props.wait_type === 'string' ? props.wait_type : 'delay';
    const target =
      (typeof props.duration === 'string' && props.duration) ||
      (typeof props.until === 'string' && props.until) ||
      (typeof props.event_type === 'string' && props.event_type) ||
      (typeof props.cron === 'string' && props.cron) ||
      '(unset)';
    detail.push(`wait ${waitType}: ${target}`);
  }
  if (kind === 'sub_flow') {
    detail.push(`sub-flow: ${refPath(props.flow_ref as never) ?? '(no flow_ref)'}`);
    if (props.async === true) detail.push('async');
  }
  if (kind === 'decision') {
    if (props.condition) detail.push(`condition: ${props.condition}`);
    // The unnamed arm falls through to the next sibling (the engine fills
    // it in during lowering), so show the effective routing.
    const yes = typeof props.yes_branch === 'string' ? props.yes_branch : successor;
    const no = typeof props.no_branch === 'string' ? props.no_branch : successor;
    detail.push(`yes → ${yes}, no → ${no}`);
  }
  if (props.retry) {
    detail.push(
      `retry: max ${props.retry.max_retries ?? '?'} (base ${props.retry.base_delay_ms ?? '?'}ms, cap ${props.retry.max_delay_ms ?? '?'}ms)`
    );
  } else if (props.retry_strategy) {
    detail.push(`retry strategy: ${props.retry_strategy}`);
  }
  const compensation = refPath(props.compensation_ref);
  if (compensation) detail.push(`compensation: ${compensation}`);
  if (props.continue_on_fail) detail.push('continue_on_fail');
  if (props.disabled) detail.push('disabled');

  // A decision step routes; record the arms so reachability and cycle
  // analysis follow BOTH of them, not just the sequential successor.
  const routing =
    kind === 'decision'
      ? {
          condition: typeof props.condition === 'string' ? props.condition : undefined,
          yes: typeof props.yes_branch === 'string' ? props.yes_branch : successor,
          no: typeof props.no_branch === 'string' ? props.no_branch : successor,
        }
      : {};

  out.set(id, {
    id,
    kind,
    label: props.action,
    detail,
    next: successor,
    ...routing,
    errorEdge: node.error_edge ?? props.error_edge,
    timeoutEdge: props.timeout_edge,
  });
  return id;
}

function lowerContainer(node: DesignerNode, successor: string, out: Map<string, PlanNode>): string {
  const id = node.id ?? '<missing-id>';
  const children = node.children ?? [];

  switch (node.container_type) {
    case 'and': {
      const entry = lowerNodes(children, successor, out);
      out.set(id, { id, kind: 'sequence', detail: ['and container (pass-through)'], next: entry });
      return id;
    }

    case 'or': {
      for (const child of children) lowerNode(child, successor, out);

      let effective: Array<{ condition: string; target: string }> = (node.rules ?? [])
        .filter((r) => r.next_step != null)
        .map((r) => ({ condition: r.condition ?? '', target: r.next_step! }));
      if (effective.length === 0) {
        for (const child of children) {
          if (isStep(child) && child.properties?.condition) {
            effective.push({ condition: child.properties.condition, target: child.id ?? '<missing-id>' });
          }
        }
      }

      if (effective.length === 0) {
        const target = children.length > 0 ? (children[0].id ?? '<missing-id>') : successor;
        out.set(id, { id, kind: 'sequence', detail: ['or container with no rules (passes through to first child)'], next: target });
        return id;
      }

      let fallthrough = successor;
      for (let idx = effective.length - 1; idx >= 0; idx--) {
        const nodeId = idx === 0 ? id : `${id}__rule${idx}`;
        out.set(nodeId, {
          id: nodeId,
          kind: 'decision',
          detail: [],
          condition: effective[idx].condition,
          yes: effective[idx].target,
          no: fallthrough,
        });
        fallthrough = nodeId;
      }
      return id;
    }

    case 'parallel': {
      const mergeStrategy =
        typeof node.merge_strategy === 'string' ? node.merge_strategy : 'merge_all';
      const fanOut = node.fan_out;

      if (fanOut != null) {
        // Fan-out: the children are ONE branch subgraph instantiated per
        // collection item, so the branch COUNT is only known at run time.
        const entry = lowerNodes(children, 'end', out);
        const detail = [
          `fan-out over: ${typeof fanOut.over === 'string' ? fanOut.over : '(missing)'}`,
          `1 branch per item, merge strategy: ${mergeStrategy}`,
        ];
        if (fanOut.max_branches != null) detail.push(`max_branches: ${fanOut.max_branches}`);
        out.set(id, {
          id,
          kind: 'parallel',
          detail,
          next: successor,
          branches: entry === 'end' ? [] : [{ id: `${id}-\${index}`, entry }],
        });
        return id;
      }

      const branches: PlanBranch[] = [];
      for (const child of children) {
        // Branch flows are independent child flows that end at their own end
        const entry = lowerNode(child, 'end', out);
        branches.push({ id: child.id ?? '<missing-id>', entry });
      }
      out.set(id, {
        id,
        kind: 'parallel',
        detail: [`${branches.length} branch(es), merge strategy: ${mergeStrategy}`],
        next: successor,
        branches,
      });
      return id;
    }

    case 'loop': {
      // Body chains back to the loop node (back-edge); the loop node
      // advances the iteration and exits to the successor when done.
      const entry = lowerNodes(children, id, out);
      const cfg = node.loop ?? {};
      // Branch on the SHAPE. This used to print `over: (missing)` for anything
      // without a collection, which is now how a legitimate `while` or `times`
      // loop renders — an explain that calls a valid flow broken is worse than
      // no explain.
      const detail: string[] = [];
      if (typeof cfg.while === 'string' && cfg.while !== '') {
        detail.push(`while: ${cfg.while}`);
        if (cfg.unbounded === true) detail.push('unbounded (no iteration ceiling)');
      } else if (typeof cfg.times === 'number') {
        detail.push(`times: ${cfg.times}`);
      } else {
        detail.push(`over: ${typeof cfg.over === 'string' ? cfg.over : '(missing)'}`);
      }
      detail.push(`item: ${typeof cfg.item === 'string' && cfg.item !== '' ? cfg.item : 'item'}`);
      if (cfg.max_iterations != null) detail.push(`max_iterations: ${cfg.max_iterations}`);
      if (typeof cfg.until === 'string' && cfg.until !== '') detail.push(`until: ${cfg.until}`);
      out.set(id, {
        id,
        kind: 'loop',
        detail,
        next: successor,
        body: entry === id ? undefined : entry,
      });
      return id;
    }

    case 'ai_sequence':
    default: {
      const detail: string[] = [];
      if (node.container_type === 'ai_sequence') {
        const agent = refPath(node.ai_config?.agent_ref ?? undefined);
        detail.push(`agent: ${agent ?? '(missing)'}`);
        detail.push(`tool_mode: ${node.ai_config?.tool_mode ?? 'auto'}, max_iterations: ${node.ai_config?.max_iterations ?? 10}`);
      } else {
        detail.push(`unknown container_type: ${node.container_type ?? '(none)'}`);
      }
      const tools: string[] = [];
      for (const child of children) {
        tools.push(lowerNode(child, 'end', out));
      }
      out.set(id, { id, kind: 'ai_container', detail, next: successor, tools });
      return id;
    }
  }
}

function lowerNode(node: DesignerNode, successor: string, out: Map<string, PlanNode>): string {
  if (isContainer(node)) return lowerContainer(node, successor, out);
  return lowerStep(node, successor, out);
}

function lowerNodes(nodes: DesignerNode[], successor: string, out: Map<string, PlanNode>): string {
  let next = successor;
  for (let i = nodes.length - 1; i >= 0; i--) {
    next = lowerNode(nodes[i], next, out);
  }
  return next;
}

/** Lower a designer flow into the summary execution plan. */
export function lowerFlow(def: DesignerFlowDefinition): LoweredFlow {
  const nodes = new Map<string, PlanNode>();
  const entry = lowerNodes(def.nodes ?? [], 'end', nodes);
  // Inject the implicit caps ONLY where the author has not claimed those ids —
  // mirroring the engine's lowering. Setting them unconditionally OVERWROTE an
  // author-declared `start`/`end` in the plan map, so `flow explain` rendered
  // the wrong graph for exactly the flows the engine now supports.
  if (!nodes.has('start')) {
    nodes.set('start', { id: 'start', kind: 'start', detail: [], next: entry });
  }
  if (!nodes.has('end')) {
    nodes.set('end', { id: 'end', kind: 'end', detail: [] });
  }
  return { entry, nodes };
}

/** All outgoing edges of a plan node (normal + error/timeout + branches). */
export function planEdges(node: PlanNode, plan: LoweredFlow): string[] {
  const out: string[] = [];
  const push = (id?: string) => {
    if (id && plan.nodes.has(id)) out.push(id);
  };
  push(node.next);
  push(node.yes);
  push(node.no);
  push(node.body);
  push(node.errorEdge);
  push(node.timeoutEdge);
  for (const b of node.branches ?? []) push(b.entry);
  for (const t of node.tools ?? []) push(t);
  return out;
}

/** Is `target` reachable from `from` in the lowered graph? */
export function isReachable(plan: LoweredFlow, from: string, target: string): boolean {
  const visited = new Set<string>();
  const stack = [from];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (id === target) return true;
    if (visited.has(id)) continue;
    visited.add(id);
    const node = plan.nodes.get(id);
    if (node) stack.push(...planEdges(node, plan));
  }
  return false;
}
