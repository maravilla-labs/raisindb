/**
 * Flow discovery and parsing.
 *
 * Accepts a single flow file (YAML/JSON, or JS with an embedded
 * workflow_data literal) or a package folder which is scanned recursively.
 */

import fs from 'fs';
import path from 'path';
import vm from 'vm';
import yaml from 'yaml';
import type { DesignerFlowDefinition, DiscoveryResult, FlowSource, ParseFailure } from './types.js';

const SKIP_DIRS = new Set(['node_modules', '.git', 'dist', 'target', '.svn', '.idea', '.vscode']);

/** Heuristics: does this parsed document contain a flow definition? */
function extractDefinition(doc: unknown): { def: unknown; name?: string } | null {
  if (doc == null || typeof doc !== 'object' || Array.isArray(doc)) return null;
  const obj = doc as Record<string, unknown>;

  // Content node file: node_type raisin:Flow with properties.workflow_data
  if (obj.node_type === 'raisin:Flow') {
    const props = (obj.properties ?? {}) as Record<string, unknown>;
    const data = props.workflow_data ?? props.flow_definition;
    const name =
      (typeof props.name === 'string' && props.name) ||
      (typeof props.title === 'string' && props.title) ||
      undefined;
    return { def: data ?? null, name };
  }

  // Standalone definition: a top-level nodes array
  if (Array.isArray(obj.nodes)) {
    return { def: obj };
  }
  return null;
}

function looksLikeDesigner(def: DesignerFlowDefinition): boolean {
  const nodes = def.nodes ?? [];
  return nodes.every((n) => typeof n === 'object' && n != null && 'node_type' in n);
}

function looksLikeRuntime(def: DesignerFlowDefinition): boolean {
  const nodes = (def.nodes ?? []) as Array<Record<string, unknown>>;
  return nodes.some((n) => typeof n === 'object' && n != null && 'step_type' in n && !('node_type' in n));
}

/**
 * Classify a parsed workflow_data value as a designer/runtime flow source.
 * Returns null when the value is not a recognizable flow definition.
 * Exported so package validation can reuse the doctor's classification.
 */
export function classifyDefinition(file: string, def: unknown, name: string, note?: string): FlowSource | null {
  if (def == null || typeof def !== 'object' || !Array.isArray((def as DesignerFlowDefinition).nodes)) {
    return null;
  }
  const typed = def as DesignerFlowDefinition;
  if (looksLikeRuntime(typed)) {
    return {
      file,
      name,
      format: 'runtime',
      note: note ? `${note}; runtime format (not analyzed by doctor)` : 'runtime format (not analyzed by doctor)',
    };
  }
  if (looksLikeDesigner(typed)) {
    return { file, name, format: 'designer', definition: typed, note };
  }
  return null;
}

function parseYamlFile(file: string, failures: ParseFailure[]): FlowSource | null {
  let doc: unknown;
  try {
    doc = yaml.parse(fs.readFileSync(file, 'utf-8'));
  } catch (error) {
    failures.push({ file, error: error instanceof Error ? error.message : String(error) });
    return null;
  }
  const extracted = extractDefinition(doc);
  if (!extracted) return null;
  const fallbackName = path.basename(path.dirname(file));
  const name =
    extracted.name ??
    (path.basename(file).startsWith('.node.') ? fallbackName : path.basename(file).replace(/\.(ya?ml|json)$/i, ''));
  if (extracted.def == null) {
    failures.push({ file, error: 'raisin:Flow node has no workflow_data property' });
    return null;
  }
  const source = classifyDefinition(file, extracted.def, name);
  if (!source) {
    failures.push({ file, error: 'workflow_data is not a recognizable flow definition (no nodes array)' });
  }
  return source;
}

/**
 * Extract `workflow_data:` / `workflowData =` object literals from a JS
 * source file without executing the module. The literal is evaluated in an
 * empty vm sandbox; literals referencing module variables are skipped.
 */
export function extractFromJs(file: string): FlowSource[] {
  const text = fs.readFileSync(file, 'utf-8');
  const sources: FlowSource[] = [];
  const pattern = /workflow(?:_data|Data)\s*[:=]\s*\{/g;
  let match: RegExpExecArray | null;
  let index = 0;
  while ((match = pattern.exec(text)) !== null) {
    const start = match.index + match[0].length - 1; // position of '{'
    const literal = extractBalanced(text, start);
    if (!literal) continue;
    let value: unknown;
    try {
      value = vm.runInNewContext(`(${literal})`, Object.create(null), { timeout: 1000 });
    } catch {
      continue; // references module-level variables; cannot analyze statically
    }
    index += 1;
    const name = `${path.basename(file)}#${index}`;
    const source = classifyDefinition(file, value, name, 'extracted from JS source');
    if (source) sources.push(source);
  }
  return sources;
}

/** Extract a balanced {...} literal, skipping string AND comment contents
 * (an apostrophe inside a // comment must not open a phantom string). */
function extractBalanced(text: string, start: number): string | null {
  let depth = 0;
  let quote: string | null = null;
  for (let i = start; i < text.length; i++) {
    const ch = text[i];
    if (quote) {
      if (ch === '\\') i += 1;
      else if (ch === quote) quote = null;
      continue;
    }
    if (ch === '/' && text[i + 1] === '/') {
      const nl = text.indexOf('\n', i);
      if (nl === -1) return null;
      i = nl;
      continue;
    }
    if (ch === '/' && text[i + 1] === '*') {
      const end = text.indexOf('*/', i + 2);
      if (end === -1) return null;
      i = end + 1;
      continue;
    }
    if (ch === "'" || ch === '"' || ch === '`') quote = ch;
    else if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  return null;
}

function scanDirectory(dir: string, sources: FlowSource[], failures: ParseFailure[]): void {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) scanDirectory(full, sources, failures);
      continue;
    }
    if (/\.(ya?ml|json)$/i.test(entry.name) || entry.name === '.node.yaml') {
      const source = parseYamlFile(full, failures);
      if (source) sources.push(source);
    } else if (/\.(mjs|cjs|js)$/i.test(entry.name)) {
      try {
        sources.push(...extractFromJs(full));
      } catch {
        // unreadable JS file: ignore
      }
    }
  }
}

/**
 * Discover flow definitions at `target` (file or folder).
 */
export function discoverFlows(target: string): DiscoveryResult {
  const resolved = path.resolve(target);
  if (!fs.existsSync(resolved)) {
    return { sources: [], failures: [{ file: resolved, error: 'path not found' }] };
  }

  const failures: ParseFailure[] = [];
  const sources: FlowSource[] = [];

  if (fs.statSync(resolved).isDirectory()) {
    scanDirectory(resolved, sources, failures);
    return { sources, failures, packageDir: resolved };
  }

  if (/\.(mjs|cjs|js)$/i.test(resolved)) {
    const extracted = extractFromJs(resolved);
    if (extracted.length === 0) {
      failures.push({ file: resolved, error: 'no statically analyzable workflow_data literal found' });
    }
    return { sources: extracted, failures };
  }

  const source = parseYamlFile(resolved, failures);
  if (!source && failures.length === 0) {
    failures.push({ file: resolved, error: 'not a flow definition (expected raisin:Flow node or designer nodes array)' });
  }
  if (source) sources.push(source);
  return { sources, failures };
}
