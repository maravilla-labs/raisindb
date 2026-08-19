import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { fileURLToPath } from 'url';
import { checkFlow } from './checks.js';
import { runDoctor } from './doctor.js';
import { lowerFlow } from './lower.js';
import { extractTemplates, expressionRoots } from './template-check.js';
import type { DesignerFlowDefinition, Finding } from './types.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../../../..');

function codes(findings: Finding[]): string[] {
  return findings.map((f) => f.code);
}

function check(def: DesignerFlowDefinition): Finding[] {
  return checkFlow(def, null);
}

function step(id: string, properties: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) {
  return { id, node_type: 'raisin:FlowStep', properties: { action: `Step ${id}`, ...properties }, ...extra };
}

function fnStep(id: string, ref = `/lib/${id}`, properties: Record<string, unknown> = {}) {
  return step(id, { function_ref: ref, ...properties });
}

describe('flow doctor: valid flows', () => {
  it('accepts the docs/workflows.md worked example (order fulfillment)', () => {
    const def: DesignerFlowDefinition = {
      version: 1,
      error_strategy: 'fail_fast',
      nodes: [
        fnStep('validate', '/lib/validate-order', {
          arguments: { order_id: '{{ input.order_id }}', items: '${input.items}' },
          retry: { max_retries: 2, base_delay_ms: 1000, max_delay_ms: 10000 },
          error_edge: 'record-failure',
        }),
        {
          id: 'route',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          rules: [
            { condition: 'input.amount >= 1000', next_step: 'approve-large' },
            { condition: 'input.amount < 1000', next_step: 'auto-approve' },
          ],
          children: [
            step('approve-large', {
              action: 'Approve order {{ input.order_id }} ({{ input.amount }} CHF)',
              step_type: 'human_task',
              task_type: 'approval',
              assignee: '/agents/order-approver',
              min_confidence: 0.8,
              escalation_assignee: '/users/ops-lead',
              task_description: 'Validated: {{ steps.validate.summary }}',
              priority: 4,
              due_in_seconds: 86400,
              timeout_edge: 'record-failure',
              options: [
                { value: 'approve', label: 'Approve', style: 'success' },
                { value: 'reject', label: 'Reject', style: 'danger' },
              ],
            }),
            fnStep('auto-approve', '/lib/mark-approved', { arguments: { order_id: '{{ input.order_id }}' } }),
          ],
        },
        {
          id: 'decision-gate',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          rules: [
            { condition: '__human_response.action == "reject"', next_step: 'record-rejection' },
            { condition: 'true', next_step: 'charge' },
          ],
          children: [
            fnStep('record-rejection', '/lib/record-rejection', {
              arguments: { reason: '{{ __human_response.comment }}' },
            }),
            fnStep('charge', '/lib/charge-payment', {
              arguments: { amount: '${input.amount}' },
              compensation_ref: '/lib/refund-payment',
              error_edge: 'record-failure',
            }),
          ],
        },
        fnStep('ship'),
        fnStep('notify', '/lib/send-notification', {
          arguments: { tracking: '{{ steps.ship.tracking_number }}' },
          continue_on_fail: true,
        }),
        fnStep('record-failure', '/lib/record-failure', {
          arguments: { failed_step: '{{ error.step_id }}', message: '{{ error.message }}' },
        }),
      ],
    };
    const findings = check(def);
    expect(findings).toEqual([]);
  });

  it('accepts the approval-flow example definition (human approval task)', () => {
    const def: DesignerFlowDefinition = {
      version: 1,
      error_strategy: 'fail_fast',
      nodes: [
        step('approve', {
          action: 'Approve order {{ input.order_id }} ({{ input.amount }} CHF)',
          step_type: 'human_task',
          task_type: 'approval',
          assignee: '/users/admin',
          task_description: 'A new order needs your approval before fulfillment.',
          priority: 4,
          options: [
            { value: 'approve', label: 'Approve', style: 'success' },
            { value: 'reject', label: 'Reject', style: 'danger' },
          ],
        }),
      ],
    };
    expect(check(def)).toEqual([]);
  });

  it('accepts an ai_sequence container with $auto agent and no children (builtin ai-agent-handler)', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'ai-agent',
          node_type: 'raisin:FlowContainer',
          container_type: 'ai_sequence',
          ai_config: { agent_ref: '$auto', tool_mode: 'auto', max_iterations: 10 },
          children: [],
        },
      ],
    };
    expect(check(def).filter((f) => f.severity === 'error')).toEqual([]);
  });
});

describe('flow doctor: schema/shape checks', () => {
  it('flags unknown node_type', () => {
    const def = { nodes: [{ id: 'x', node_type: 'raisin:Bogus' }] };
    expect(codes(check(def))).toContain('UNKNOWN_NODE_TYPE');
  });

  it('flags unknown step_type and container_type', () => {
    const def = {
      nodes: [
        step('a', { step_type: 'mystery', function_ref: '/lib/a' }),
        { id: 'c', node_type: 'raisin:FlowContainer', container_type: 'xor', children: [fnStep('b')] },
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('UNKNOWN_STEP_TYPE');
    expect(found).toContain('UNKNOWN_CONTAINER_TYPE');
  });

  it('flags duplicate, empty, and reserved ids', () => {
    const def = {
      nodes: [
        fnStep('dup'),
        fnStep('dup'),
        { node_type: 'raisin:FlowStep', properties: { action: 'x', function_ref: '/lib/x' } },
        fnStep('start'),
        fnStep('__internal'),
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('DUPLICATE_NODE_ID');
    expect(found).toContain('EMPTY_NODE_ID');
    expect(found.filter((c) => c === 'RESERVED_NODE_ID')).toHaveLength(2);
  });

  it('flags an empty flow', () => {
    expect(codes(check({ nodes: [] }))).toContain('EMPTY_FLOW');
  });
});

describe('flow doctor: edge integrity', () => {
  it('flags error_edge / timeout_edge pointing to missing nodes', () => {
    const def = {
      nodes: [
        fnStep('a', '/lib/a', { error_edge: 'nope' }),
        step('t', {
          step_type: 'human_task',
          task_type: 'approval',
          assignee: '/users/x',
          options: [{ value: 'ok', label: 'OK' }],
          timeout_edge: 'gone',
        }),
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('INVALID_ERROR_EDGE');
    expect(found).toContain('INVALID_TIMEOUT_EDGE');
  });

  it('accepts node-level error_edge targeting a top-level node', () => {
    const def = {
      nodes: [fnStep('a', '/lib/a', {}, ), { ...fnStep('b'), error_edge: 'a' }],
    };
    expect(codes(check(def))).not.toContain('INVALID_ERROR_EDGE');
  });

  it('flags rules routing to non-children', () => {
    const def = {
      nodes: [
        {
          id: 'or1',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          rules: [{ condition: 'input.x == 1', next_step: 'outside' }],
          children: [fnStep('inside', '/lib/inside', { condition: 'input.x == 1' })],
        },
        fnStep('outside'),
      ],
    };
    expect(codes(check(def))).toContain('INVALID_RULE_TARGET');
  });
});

describe('flow doctor: template expressions', () => {
  it('extracts both marker styles and detects unbalanced markers', () => {
    expect(extractTemplates('Order {{ input.id }} for ${input.user}').expressions).toEqual([
      'input.id',
      'input.user',
    ]);
    expect(extractTemplates('price is ${100').unbalanced).toBe(true);
    expect(extractTemplates('hello {{ input.x').unbalanced).toBe(true);
  });

  it('extracts roots and hyphenated step refs', () => {
    const { roots, stepRefs, hyphenDotRefs } = expressionRoots(
      'steps.book-flight.arrival + input.offset'
    );
    expect(stepRefs).toEqual(['book-flight']);
    expect(hyphenDotRefs).toEqual(['book-flight']);
    expect(roots).toEqual(['steps', 'input']);
  });

  it('extracts bracket-access step refs without flagging them as REL traps', () => {
    const { stepRefs, hyphenDotRefs } = expressionRoots(
      "steps['book-flight'].arrival + input.offset"
    );
    expect(stepRefs).toEqual(['book-flight']);
    expect(hyphenDotRefs).toEqual([]);
  });

  it('flags dot access on hyphenated step ids as a REL subtraction trap', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        fnStep('create-accounts', '/lib/create-accounts'),
        fnStep('send-welcome', '/lib/send-welcome', {
          arguments: {
            broken: '${steps.create-accounts.email}',
            ok: "${steps['create-accounts'].email}",
          },
        }),
      ],
    };
    const findings = check(def);
    const hyphen = findings.filter((f) => f.code === 'TEMPLATE_HYPHENATED_STEP_PATH');
    expect(hyphen).toHaveLength(1);
    expect(hyphen[0].nodeId).toBe('send-welcome');
    expect(hyphen[0].severity).toBe('error');
    // The bracket form is valid and gets normal step validation (no unknown-step)
    expect(findings.filter((f) => f.code === 'TEMPLATE_UNKNOWN_STEP')).toHaveLength(0);
  });

  it('flags unknown roots, unknown steps, and forward refs', () => {
    const def = {
      nodes: [
        fnStep('first', '/lib/first', {
          arguments: {
            bad_root: '{{ stuff.x }}',
            missing_step: '{{ steps.ghost.value }}',
            forward: '{{ steps.second.value }}',
          },
        }),
        fnStep('second'),
      ],
    };
    const findings = check(def);
    const found = codes(findings);
    expect(found).toContain('TEMPLATE_UNKNOWN_ROOT');
    expect(found).toContain('TEMPLATE_UNKNOWN_STEP');
    expect(found).toContain('TEMPLATE_FORWARD_REF');
    expect(findings.find((f) => f.code === 'TEMPLATE_FORWARD_REF')?.severity).toBe('error');
  });

  it('flags unbalanced templates as warnings', () => {
    const def = { nodes: [fnStep('a', '/lib/a', { arguments: { x: 'price is ${100' } })] };
    const finding = check(def).find((f) => f.code === 'TEMPLATE_UNBALANCED');
    expect(finding?.severity).toBe('warning');
  });

  it('accepts backward steps refs and all known namespaces', () => {
    const def = {
      nodes: [
        fnStep('a'),
        fnStep('b', '/lib/b', {
          arguments: {
            prev: '{{ steps.a.result }}',
            inp: '{{ input.x }}',
            trg: '{{ trigger.event_type }}',
            flw: '{{ flow.instance_id }}',
            err: '{{ error.message }}',
            hum: '{{ __human_response.action }}',
          },
        }),
      ],
    };
    expect(check(def)).toEqual([]);
  });
});

describe('flow doctor: human tasks', () => {
  it('flags a missing task_type and a missing assignee', () => {
    const def = {
      nodes: [step('t1', { step_type: 'human_task' })],
    };
    const found = codes(check(def));
    expect(found).toContain('MISSING_TASK_TYPE');
    expect(found).toContain('MISSING_ASSIGNEE');
  });

  it('accepts an application-defined task_type slug', () => {
    // `task_type` is an OPEN set: approval/input/review/action are the types
    // the runtime understands semantically, but any slug is valid and is
    // carried through verbatim. This used to report UNKNOWN_TASK_TYPE from a
    // closed enum that lived in four places and drifted between them.
    const def = {
      nodes: [
        step('t2', {
          step_type: 'human_task',
          task_type: 'celebration',
          assignee: '/users/a',
        }),
      ],
    };
    expect(codes(check(def))).toEqual([]);
  });

  it('flags a task_type that is not a valid slug', () => {
    // Shape is checked, membership is not.
    const def = {
      nodes: [
        step('t3', {
          step_type: 'human_task',
          task_type: 'Not A Slug',
          assignee: '/users/a',
        }),
      ],
    };
    expect(codes(check(def))).toContain('INVALID_TASK_TYPE');
  });

  it('warns on approval without options and input without schema', () => {
    const def = {
      nodes: [
        step('a', { step_type: 'human_task', task_type: 'approval', assignee: '/users/x' }),
        step('i', { step_type: 'human_task', task_type: 'input', assignee: '/users/x' }),
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('MISSING_TASK_OPTIONS');
    expect(found).toContain('MISSING_INPUT_SCHEMA');
  });

  it('suggests guardrails for agent assignees and validates min_confidence', () => {
    const def = {
      nodes: [
        step('a', {
          step_type: 'human_task',
          task_type: 'approval',
          assignee: '/agents/approver',
          options: [{ value: 'ok', label: 'OK' }],
        }),
        step('b', {
          step_type: 'human_task',
          task_type: 'approval',
          assignee: '/agents/approver',
          min_confidence: 1.5,
          escalation_assignee: '/users/boss',
          options: [{ value: 'ok', label: 'OK' }],
        }),
      ],
    };
    const findings = check(def);
    const guardrail = findings.find((f) => f.code === 'AGENT_ASSIGNEE_GUARDRAILS');
    expect(guardrail?.severity).toBe('suggestion');
    expect(guardrail?.nodeId).toBe('a');
    expect(codes(findings)).toContain('INVALID_MIN_CONFIDENCE');
  });
});

describe('flow doctor: OR containers', () => {
  it('errors when an OR container has no routing information', () => {
    const def = {
      nodes: [
        {
          id: 'or1',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          children: [fnStep('a'), fnStep('b')],
        },
      ],
    };
    expect(codes(check(def))).toContain('OR_UNROUTABLE');
  });

  it('warns about unreachable children', () => {
    const def = {
      nodes: [
        {
          id: 'or1',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          rules: [{ condition: 'input.x == 1', next_step: 'a' }],
          children: [fnStep('a'), fnStep('b')],
        },
      ],
    };
    const finding = check(def).find((f) => f.code === 'UNROUTED_OR_CHILD');
    expect(finding?.nodeId).toBe('b');
  });
});

describe('flow doctor: retry / containers / conditions', () => {
  it('warns on suspicious retry config and errors on unknown strategy', () => {
    const def = {
      nodes: [
        fnStep('a', '/lib/a', { retry: { max_retries: 2, base_delay_ms: 5000, max_delay_ms: 1000 } }),
        fnStep('b', '/lib/b', { retry_strategy: 'turbo' }),
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('INVALID_RETRY_CONFIG');
    expect(found).toContain('UNKNOWN_RETRY_STRATEGY');
  });

  it('warns on empty containers and nested parallel; errors on ai_sequence without agent', () => {
    const def = {
      nodes: [
        { id: 'empty', node_type: 'raisin:FlowContainer', container_type: 'and', children: [] },
        {
          id: 'outer',
          node_type: 'raisin:FlowContainer',
          container_type: 'parallel',
          children: [
            { id: 'inner', node_type: 'raisin:FlowContainer', container_type: 'parallel', children: [fnStep('x')] },
          ],
        },
        { id: 'ai1', node_type: 'raisin:FlowContainer', container_type: 'ai_sequence', children: [] },
        { id: 'ai2', node_type: 'raisin:FlowContainer', container_type: 'ai_sequence', ai_config: {}, children: [] },
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('EMPTY_CONTAINER');
    expect(found).toContain('NESTED_PARALLEL');
    expect(found).toContain('MISSING_AI_CONFIG');
    expect(found).toContain('MISSING_AI_AGENT_REF');
  });

  it('flags empty and unbalanced conditions', () => {
    const def = {
      nodes: [
        step('a', { condition: '   ' }),
        step('b', { condition: '(input.x == "open"' }),
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('EMPTY_CONDITION');
    expect(found).toContain('INVALID_CONDITION');
  });
});

describe('flow doctor: cycles', () => {
  it('warns when an error edge can loop back to its own step', () => {
    const def = {
      nodes: [fnStep('b'), fnStep('a', '/lib/a', { error_edge: 'b' })],
    };
    // a fails -> b; b's normal successor is a (sibling order) -> cycle
    const finding = check(def).find((f) => f.code === 'POSSIBLE_CYCLE');
    expect(finding?.nodeId).toBe('a');
    expect(finding?.severity).toBe('warning');
  });

  it('does not warn for a forward error handler', () => {
    const def = {
      nodes: [fnStep('a', '/lib/a', { error_edge: 'handler' }), fnStep('b'), fnStep('handler')],
    };
    expect(codes(check(def))).not.toContain('POSSIBLE_CYCLE');
  });
});

describe('flow lowering (explain)', () => {
  it('chains siblings, builds OR decision cascades and parallel branches', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        fnStep('first'),
        {
          id: 'route',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          rules: [
            { condition: 'input.a == 1', next_step: 'x' },
            { condition: 'input.a == 2', next_step: 'y' },
          ],
          children: [fnStep('x'), fnStep('y')],
        },
        {
          id: 'par',
          node_type: 'raisin:FlowContainer',
          container_type: 'parallel',
          children: [fnStep('left'), fnStep('right')],
        },
        fnStep('last'),
      ],
    };
    const plan = lowerFlow(def);
    expect(plan.nodes.get('start')?.next).toBe('first');
    expect(plan.nodes.get('first')?.next).toBe('route');
    // OR cascade: container id is rule 0, falls through to __rule1, then to successor
    const rule0 = plan.nodes.get('route')!;
    expect(rule0.kind).toBe('decision');
    expect(rule0.yes).toBe('x');
    expect(rule0.no).toBe('route__rule1');
    expect(plan.nodes.get('route__rule1')?.yes).toBe('y');
    expect(plan.nodes.get('route__rule1')?.no).toBe('par');
    // OR children exit to the container successor
    expect(plan.nodes.get('x')?.next).toBe('par');
    // Parallel branches
    const par = plan.nodes.get('par')!;
    expect(par.kind).toBe('parallel');
    expect(par.branches).toEqual([
      { id: 'left', entry: 'left' },
      { id: 'right', entry: 'right' },
    ]);
    expect(par.next).toBe('last');
    expect(plan.nodes.get('last')?.next).toBe('end');
  });
});

describe('flow doctor: package folder mode', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'flow-doctor-test-'));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  function write(rel: string, content: string) {
    const full = path.join(tmpDir, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content);
  }

  const fnYaml = [
    'node_type: raisin:Function',
    'properties:',
    '  language: javascript',
    '  entry_file: index.js:main',
    '  input_schema:',
    '    type: object',
    '    required: [order_id, amount]',
    '    properties:',
    '      order_id: { type: string }',
    '      amount: { type: number }',
  ].join('\n');

  function flowYaml(args: string) {
    return [
      'node_type: raisin:Flow',
      'properties:',
      '  name: test-flow',
      '  enabled: true',
      '  workflow_data:',
      '    version: 1',
      '    nodes:',
      '      - id: charge',
      '        node_type: raisin:FlowStep',
      '        properties:',
      '          action: Charge',
      '          function_ref: /lib/charge',
      args,
    ].join('\n');
  }

  it('resolves function refs and checks required arguments', () => {
    write('functions/lib/charge/.node.yaml', fnYaml);
    write(
      'functions/flows/test-flow/.node.yaml',
      flowYaml('          arguments: { order_id: "{{ input.order_id }}" }')
    );
    const result = runDoctor(tmpDir);
    const findings = result.reports.flatMap((r) => r.findings);
    const missing = findings.filter((f) => f.code === 'MISSING_REQUIRED_ARGUMENT');
    expect(missing).toHaveLength(1);
    expect(missing[0].message).toContain('amount');
    expect(findings.map((f) => f.code)).not.toContain('FUNCTION_NOT_FOUND');
    expect(result.exitCode).toBe(1);
  });

  it('passes when all required arguments are provided as templates', () => {
    write('functions/lib/charge/.node.yaml', fnYaml);
    write(
      'functions/flows/test-flow/.node.yaml',
      flowYaml('          arguments: { order_id: "{{ input.order_id }}", amount: "${input.amount}" }')
    );
    const result = runDoctor(tmpDir);
    expect(result.reports.flatMap((r) => r.findings)).toEqual([]);
    expect(result.exitCode).toBe(0);
  });

  it('warns when a function ref does not resolve in the package', () => {
    write(
      'functions/flows/test-flow/.node.yaml',
      flowYaml('          arguments: {}').replace('/lib/charge', '/lib/missing')
    );
    const result = runDoctor(tmpDir);
    const findings = result.reports.flatMap((r) => r.findings);
    expect(findings.map((f) => f.code)).toContain('FUNCTION_NOT_FOUND');
    expect(findings.find((f) => f.code === 'FUNCTION_NOT_FOUND')?.severity).toBe('warning');
    expect(result.exitCode).toBe(0); // warning only
    expect(runDoctor(tmpDir, { strict: true }).exitCode).toBe(1);
  });

  it('errors when a ref resolves to a non-function node', () => {
    write('functions/lib/charge/.node.yaml', 'node_type: raisin:Folder\nproperties: {}');
    write('functions/flows/test-flow/.node.yaml', flowYaml('          arguments: {}'));
    const result = runDoctor(tmpDir);
    expect(result.reports.flatMap((r) => r.findings).map((f) => f.code)).toContain('FUNCTION_NOT_A_FUNCTION');
    expect(result.exitCode).toBe(1);
  });

  it('errors on invalid reference shape', () => {
    write(
      'functions/flows/test-flow/.node.yaml',
      [
        'node_type: raisin:Flow',
        'properties:',
        '  name: test-flow',
        '  workflow_data:',
        '    nodes:',
        '      - id: charge',
        '        node_type: raisin:FlowStep',
        '        properties:',
        '          action: Charge',
        '          function_ref:',
        '            raisin:workspace: functions',
      ].join('\n')
    );
    const result = runDoctor(tmpDir);
    expect(result.reports.flatMap((r) => r.findings).map((f) => f.code)).toContain('INVALID_REFERENCE');
    expect(result.exitCode).toBe(1);
  });

  it('exits 2 on unparseable YAML', () => {
    write('functions/flows/broken/.node.yaml', 'node_type: [unclosed');
    const result = runDoctor(tmpDir);
    expect(result.exitCode).toBe(2);
    expect(result.failures).toHaveLength(1);
  });

  it('skips runtime-format definitions with a note', () => {
    write(
      'flow.yaml',
      [
        'node_type: raisin:Flow',
        'properties:',
        '  name: runtime-flow',
        '  workflow_data:',
        '    nodes:',
        '      - { id: start, step_type: start, next_node: end }',
        '      - { id: end, step_type: end }',
      ].join('\n')
    );
    const result = runDoctor(tmpDir);
    expect(result.reports).toHaveLength(1);
    expect(result.reports[0].source.format).toBe('runtime');
    expect(result.reports[0].findings).toEqual([]);
    expect(result.exitCode).toBe(0);
  });
});

describe('flow doctor: real repository examples', () => {
  it('finds the embedded approval-flow example flow and reports it clean', () => {
    const exampleDir = path.join(REPO_ROOT, 'examples/workflows/approval-flow');
    if (!fs.existsSync(exampleDir)) return; // repo layout changed; skip
    const result = runDoctor(exampleDir);
    expect(result.failures).toEqual([]);
    const designerFlows = result.reports.filter((r) => r.source.format === 'designer');
    expect(designerFlows.length).toBeGreaterThanOrEqual(1);
    for (const report of designerFlows) {
      expect(report.findings).toEqual([]);
    }
    expect(result.exitCode).toBe(0);
  });

  it('reports the builtin ai-tools flows clean (verified shipped package)', () => {
    const pkgDir = path.join(REPO_ROOT, 'builtin-packages/ai-tools');
    if (!fs.existsSync(pkgDir)) return;
    const result = runDoctor(pkgDir);
    const errors = result.reports.flatMap((r) => r.findings).filter((f) => f.severity === 'error');
    expect(errors).toEqual([]);
  });
});

describe('flow doctor: AI router and competition containers', () => {
  it('accepts an OR container routed only by an AI router', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'route',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          router: { agent_ref: '/agents/dispatcher', min_confidence: 0.6, default_branch: 'b' },
          children: [step('a'), step('b')],
        },
      ],
    };
    const findings = check(def);
    expect(codes(findings)).not.toContain('OR_UNROUTABLE');
    expect(codes(findings)).not.toContain('UNROUTED_OR_CHILD');
    expect(findings.filter((f) => f.severity === 'error')).toHaveLength(0);
  });

  it('flags router problems', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'route',
          node_type: 'raisin:FlowContainer',
          container_type: 'or',
          router: { min_confidence: 1.5, default_branch: 'ghost' },
          children: [step('a'), step('b')],
        },
        {
          id: 'andbox',
          node_type: 'raisin:FlowContainer',
          container_type: 'and',
          router: { agent_ref: '/agents/x' },
          children: [step('c')],
        },
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('MISSING_ROUTER_AGENT');
    expect(found).toContain('INVALID_DEFAULT_BRANCH');
    expect(found).toContain('INVALID_MIN_CONFIDENCE');
    expect(found).toContain('ROUTER_ON_NON_OR');
  });

  it('accepts a valid competition container', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'compete',
          node_type: 'raisin:FlowContainer',
          container_type: 'competition',
          prompt: 'Write a tagline for {{ input.product }}.',
          referee: { agent_ref: '/agents/referee', min_confidence: 0.7, max_rounds: 2 },
          children: [
            step('writer_a', { step_type: 'ai_agent', agent_ref: '/agents/claude' }),
            step('writer_b', { step_type: 'ai_agent', agent_ref: '/agents/gpt' }),
          ],
        },
      ],
    };
    const findings = check(def);
    expect(findings.filter((f) => f.severity === 'error')).toHaveLength(0);
  });

  it('flags competition without referee or enough agents', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'compete',
          node_type: 'raisin:FlowContainer',
          container_type: 'competition',
          children: [
            step('only_writer', { step_type: 'ai_agent', agent_ref: '/agents/claude' }),
            step('not_an_agent'),
          ],
        },
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('COMPETITION_NEEDS_REFEREE');
    expect(found).toContain('COMPETITION_TOO_FEW_AGENTS');
    expect(found).toContain('COMPETITION_NON_AGENT_CHILD');
  });

  it('accepts a valid loop container', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        fnStep('pick_candidates'),
        {
          id: 'ask_each',
          node_type: 'raisin:FlowContainer',
          container_type: 'loop',
          loop: {
            over: '${steps.pick_candidates.candidates}',
            item: 'candidate',
            index: 'candidate_index',
            max_iterations: 10,
            until: "steps.ask.response == 'accept'",
          },
          children: [
            fnStep('ask', '/lib/ask', { arguments: { who: '${candidate}', pos: '{{ candidate_index }}' } }),
          ],
        },
      ],
    };
    const findings = check(def);
    expect(findings.filter((f) => f.severity === 'error')).toHaveLength(0);
    // The custom item/index variables must not be flagged as unknown roots
    expect(codes(findings)).not.toContain('TEMPLATE_UNKNOWN_ROOT');
  });

  it('flags loop without over and with a bad item variable', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'each',
          node_type: 'raisin:FlowContainer',
          container_type: 'loop',
          loop: { item: 'my-item', max_iterations: 0 } as never,
          children: [fnStep('body')],
        },
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('LOOP_MISSING_OVER');
    expect(found).toContain('LOOP_INVALID_VARIABLE');
    expect(found).toContain('LOOP_INVALID_MAX_ITERATIONS');
  });

  it('flags a loop container without any loop config', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'each',
          node_type: 'raisin:FlowContainer',
          container_type: 'loop',
          children: [fnStep('body')],
        },
      ],
    };
    expect(codes(check(def))).toContain('LOOP_MISSING_OVER');
  });

  it('flags until conditions referencing unknown steps and loop config on non-loop containers', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'each',
          node_type: 'raisin:FlowContainer',
          container_type: 'loop',
          loop: {
            over: '${input.items}',
            until: "steps.ghost.response == 'accept'",
          },
          children: [fnStep('body')],
        },
        {
          id: 'andbox',
          node_type: 'raisin:FlowContainer',
          container_type: 'and',
          loop: { over: '${input.items}' },
          children: [fnStep('c')],
        },
      ],
    };
    const found = codes(check(def));
    expect(found).toContain('LOOP_UNTIL_UNKNOWN_STEP');
    expect(found).toContain('LOOP_ON_NON_LOOP');
  });

  it('checks the loop.over expression like other templates', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'each',
          node_type: 'raisin:FlowContainer',
          container_type: 'loop',
          loop: { over: '${steps.ghost.items}' },
          children: [fnStep('body')],
        },
      ],
    };
    expect(codes(check(def))).toContain('TEMPLATE_UNKNOWN_STEP');
  });

  it('lowers a loop container with a body back-edge', () => {
    const def: DesignerFlowDefinition = {
      nodes: [
        {
          id: 'each',
          node_type: 'raisin:FlowContainer',
          container_type: 'loop',
          loop: { over: '${input.items}', item: 'current' },
          children: [fnStep('ask'), fnStep('record')],
        },
        fnStep('after'),
      ],
    };
    const plan = lowerFlow(def);
    const loopNode = plan.nodes.get('each')!;
    expect(loopNode.kind).toBe('loop');
    expect(loopNode.body).toBe('ask');
    expect(loopNode.next).toBe('after');
    // Body chain: ask -> record -> back to the loop node
    expect(plan.nodes.get('ask')!.next).toBe('record');
    expect(plan.nodes.get('record')!.next).toBe('each');
  });
});
