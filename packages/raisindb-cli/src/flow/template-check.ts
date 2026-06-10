/**
 * Template expression analysis for {{ expr }} and ${ expr } markers.
 *
 * Rules verified against docs/workflows.md section 4 and
 * raisin-flow-runtime/src/runtime/data_mapper.rs.
 */

import {
  KNOWN_TEMPLATE_ROOTS,
  type DesignerFlowDefinition,
  type DesignerNode,
  type Finding,
  isContainer,
  isStep,
  walkNodes,
} from './types.js';

const EXPR_KEYWORDS = new Set(['true', 'false', 'null', 'and', 'or', 'not', 'in']);

export interface TemplateScanResult {
  expressions: string[];
  unbalanced: boolean;
}

/** Extract all template expressions from a string. */
export function extractTemplates(value: string): TemplateScanResult {
  const expressions: string[] = [];
  let unbalanced = false;

  // {{ ... }} markers
  let i = 0;
  while ((i = value.indexOf('{{', i)) !== -1) {
    const close = value.indexOf('}}', i + 2);
    if (close === -1) {
      unbalanced = true;
      break;
    }
    expressions.push(value.slice(i + 2, close).trim());
    i = close + 2;
  }

  // ${ ... } markers (skip regions already captured inside {{ }})
  i = 0;
  while ((i = value.indexOf('${', i)) !== -1) {
    let depth = 1;
    let j = i + 2;
    for (; j < value.length; j++) {
      if (value[j] === '{') depth += 1;
      else if (value[j] === '}') {
        depth -= 1;
        if (depth === 0) break;
      }
    }
    if (depth !== 0) {
      unbalanced = true;
      break;
    }
    const expr = value.slice(i + 2, j).trim();
    // Avoid double-reporting "${" found inside a {{ }} expression
    if (!expressions.some((e) => e.includes(value.slice(i, j + 1)))) {
      expressions.push(expr);
    }
    i = j + 1;
  }

  return { expressions, unbalanced };
}

/** Root identifiers used in an expression (string literals stripped). */
export function expressionRoots(expr: string): {
  roots: string[];
  stepRefs: string[];
  hyphenDotRefs: string[];
} {
  const stepRefs: string[] = [];
  const hyphenDotRefs: string[] = [];
  // Bracket access (steps['my-step']) — extract BEFORE stripping string
  // literals; this is the required form for ids with non-word characters.
  for (const m of expr.matchAll(/(?<![.\w$])steps\[\s*['"]([^'"]+)['"]\s*\]/g)) {
    stepRefs.push(m[1]);
  }
  const stripped = expr.replace(/'[^']*'|"[^"]*"/g, '""');
  for (const m of stripped.matchAll(/(?<![.\w$])steps\.([\w-]+)/g)) {
    stepRefs.push(m[1]);
    // REL identifiers are [A-Za-z0-9_] only: dot access on a hyphenated id
    // parses as subtraction and silently resolves to garbage.
    if (m[1].includes('-')) hyphenDotRefs.push(m[1]);
  }
  const roots: string[] = [];
  // Negative lookbehind includes '-' so hyphenated path segments
  // (steps.book-flight.arrival) don't produce phantom roots.
  for (const m of stripped.matchAll(/(?<![.\w$-])([A-Za-z_$][A-Za-z0-9_$]*)/g)) {
    const ident = m[1];
    if (EXPR_KEYWORDS.has(ident)) continue;
    if (!roots.includes(ident)) roots.push(ident);
  }
  return { roots, stepRefs, hyphenDotRefs };
}

/** Collect every string value in a JSON-ish structure with a field path. */
function collectStrings(value: unknown, field: string, out: Array<{ field: string; text: string }>): void {
  if (typeof value === 'string') {
    out.push({ field, text: value });
  } else if (Array.isArray(value)) {
    value.forEach((v, i) => collectStrings(v, `${field}[${i}]`, out));
  } else if (value != null && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      collectStrings(v, `${field}.${k}`, out);
    }
  }
}

/** Fields scanned for template expressions on each step. */
function templatedFields(node: DesignerNode): Array<{ field: string; text: string }> {
  const props = node.properties ?? {};
  const out: Array<{ field: string; text: string }> = [];
  for (const field of ['action', 'prompt', 'task_description', 'assignee'] as const) {
    const v = props[field];
    if (typeof v === 'string') out.push({ field, text: v });
  }
  collectStrings(props.arguments, 'arguments', out);
  collectStrings(props.compensation_input_mapping, 'compensation_input_mapping', out);
  collectStrings(props.options, 'options', out);
  return out;
}

/**
 * Check template expressions across all steps:
 * - TEMPLATE_UNBALANCED: unterminated marker (runtime leaves literal text)
 * - TEMPLATE_UNKNOWN_ROOT: first path segment is not a known namespace
 * - TEMPLATE_UNKNOWN_STEP: steps.<id> does not exist
 * - TEMPLATE_FORWARD_REF: steps.<id> occurs later in document order
 */
export function checkTemplates(def: DesignerFlowDefinition, findings: Finding[]): void {
  // Document-order position of every node id (pre-order)
  const order = new Map<string, number>();
  let pos = 0;
  walkNodes(def.nodes, (node) => {
    if (typeof node.id === 'string' && !order.has(node.id)) order.set(node.id, pos);
    pos += 1;
  });

  let nodePos = -1;
  walkNodes(def.nodes, (node, ancestors) => {
    nodePos += 1;
    const isLoopContainer = isContainer(node) && node.container_type === 'loop';
    if (!isStep(node) && !isLoopContainer) return;
    const nodeId = node.id ?? '<missing-id>';

    // Loop variables (item/index) of enclosing loop containers resolve as
    // bare flow variables in the body — treat them as known roots.
    const loopRoots = loopVariableNames(ancestors);

    const fields = isLoopContainer
      ? typeof node.loop?.over === 'string'
        ? [{ field: 'loop.over', text: node.loop.over }]
        : []
      : templatedFields(node);

    for (const { field, text } of fields) {
      const { expressions, unbalanced } = extractTemplates(text);
      if (unbalanced) {
        findings.push({
          code: 'TEMPLATE_UNBALANCED',
          severity: 'warning',
          nodeId,
          field,
          message: `Unterminated template marker in "${truncate(text)}" — the runtime will keep it as literal text.`,
        });
      }
      for (const expr of expressions) {
        const { roots, stepRefs, hyphenDotRefs } = expressionRoots(expr);
        for (const stepId of hyphenDotRefs) {
          findings.push({
            code: 'TEMPLATE_HYPHENATED_STEP_PATH',
            severity: 'error',
            nodeId,
            field,
            message: `Template "${truncate(expr)}" uses dot access on hyphenated step id "${stepId}" — REL parses this as subtraction. Prefer renaming the step to "${stepId.replace(/-/g, '_')}"; bracket access (steps['${stepId}']) works but is not null-safe when the step was skipped.`,
          });
        }
        for (const root of roots) {
          if (!KNOWN_TEMPLATE_ROOTS.includes(root) && !loopRoots.includes(root)) {
            findings.push({
              code: 'TEMPLATE_UNKNOWN_ROOT',
              severity: 'warning',
              nodeId,
              field,
              message: `Template "${truncate(expr)}" starts with "${root}" which is not a known namespace (${KNOWN_TEMPLATE_ROOTS.slice(0, 8).join(', ')}). Bare flow variables resolve at runtime but are fragile.`,
            });
          }
        }
        for (const stepId of stepRefs) {
          // Hyphenated dot refs already got their dedicated error above;
          // skip the generic checks for them (the id may well exist).
          if (hyphenDotRefs.includes(stepId)) continue;
          if (!order.has(stepId)) {
            findings.push({
              code: 'TEMPLATE_UNKNOWN_STEP',
              severity: 'error',
              nodeId,
              field,
              message: `Template references steps.${stepId} but no step with id "${stepId}" exists.`,
            });
          } else if (order.get(stepId)! >= nodePos) {
            findings.push({
              code: 'TEMPLATE_FORWARD_REF',
              severity: 'error',
              nodeId,
              field,
              message: `Template references steps.${stepId} which runs after (or is) this step — its output will not exist yet.`,
            });
          }
        }
      }
    }
  });
}

/** Item/index variable names exposed by enclosing loop containers. */
function loopVariableNames(ancestors: DesignerNode[]): string[] {
  const names: string[] = [];
  for (const ancestor of ancestors) {
    if (ancestor.container_type !== 'loop') continue;
    const cfg = ancestor.loop;
    names.push(typeof cfg?.item === 'string' && cfg.item !== '' ? cfg.item : 'item');
    if (typeof cfg?.index === 'string' && cfg.index !== '') names.push(cfg.index);
  }
  return names;
}

function truncate(s: string, max = 60): string {
  return s.length > max ? `${s.slice(0, max)}…` : s;
}
