/**
 * Writing a server value into a package file without wrecking what a person
 * wrote there.
 *
 * Package YAML is hand-written: it carries comments that explain WHY a value is
 * what it is, and `{env:...}` tokens that resolve differently per environment.
 * Re-serialising the server's JSON over it would delete the comments and bake
 * one environment's URL into the package. So an existing file is UPDATED in
 * place — only the values that actually differ are replaced, an env token is
 * never replaced, and everything unchanged keeps its formatting and comments.
 */

import yaml, { Document, isMap, isScalar, isSeq, YAMLMap, YAMLSeq } from 'yaml';
import { hasEnvTokens } from '../../env/substitute.js';
import { equalModuloEnv, isPlainObject, sortKeysDeep, type Json, type JsonObject } from './normalize.js';

const STRINGIFY: yaml.ToStringOptions = { lineWidth: 0 };

/** A node file as pull writes it: `node_type`, `archetype`, `properties`. */
export interface NodeFileValue {
  node_type: string;
  archetype?: string | null;
  properties: JsonObject;
}

/** Render a fresh file — keys sorted, so the same node is always the same text. */
export function renderYaml(value: JsonObject): string {
  return yaml.stringify(value, STRINGIFY);
}

export function nodeFileObject(v: NodeFileValue): JsonObject {
  const out: JsonObject = { node_type: v.node_type };
  if (v.archetype) out.archetype = v.archetype;
  out.properties = sortKeysDeep(v.properties) as JsonObject;
  return out;
}

function keyOf(pair: { key: unknown }): string | null {
  const k = pair.key;
  if (isScalar(k)) return String(k.value);
  return typeof k === 'string' ? k : null;
}

function uuidOf(node: unknown): string | null {
  if (!isMap(node)) return null;
  const u = node.get('uuid');
  return typeof u === 'string' ? u : null;
}

/** Apply `value` onto the YAML node `node`, reusing what is unchanged. */
function apply(doc: Document, node: unknown, value: Json): unknown {
  if (isScalar(node) && typeof node.value === 'string' && hasEnvTokens(node.value)) return node;

  const current = node && typeof (node as { toJSON?: unknown }).toJSON === 'function'
    ? (node as { toJSON: () => unknown }).toJSON()
    : node;
  if (node !== undefined && node !== null && equalModuloEnv(current, value)) return node;

  if (isMap(node) && isPlainObject(value)) {
    const map = node as YAMLMap;
    const present = new Set<string>();
    map.items = map.items.filter((pair) => {
      const k = keyOf(pair);
      if (k === null) return true;
      if (!(k in value)) return false;
      present.add(k);
      pair.value = apply(doc, pair.value, value[k]) as never;
      return true;
    });
    for (const k of Object.keys(value).sort()) {
      if (!present.has(k)) map.add(doc.createPair(k, sortKeysDeep(value[k])));
    }
    return map;
  }

  if (isSeq(node) && Array.isArray(value)) {
    const seq = node as YAMLSeq;
    const byUuid = new Map<string, unknown>();
    for (const item of seq.items) {
      const u = uuidOf(item);
      if (u) byUuid.set(u, item);
    }
    const sameShape = value.length === seq.items.length;
    seq.items = value.map((v, i) => {
      const u = isPlainObject(v) && typeof v.uuid === 'string' ? v.uuid : null;
      // Blocks are matched by uuid, so moving one does not lose the comments
      // and tokens of every block after it; plain lists by position.
      const existing = u ? byUuid.get(u) : sameShape ? seq.items[i] : undefined;
      return existing !== undefined ? apply(doc, existing, v) : doc.createNode(sortKeysDeep(v));
    }) as never;
    return seq;
  }

  return doc.createNode(sortKeysDeep(value));
}

/**
 * Update an existing node file to say what the server says. Top-level keys
 * other than `node_type` / `archetype` / `properties` (an explicit `name:`,
 * an `id:`) are the package's and are left alone.
 */
export function mergeNodeYaml(existingText: string, value: NodeFileValue): string {
  const doc = yaml.parseDocument(existingText);
  if (!isMap(doc.contents)) return renderYaml(nodeFileObject(value));
  const root = doc.contents as YAMLMap;

  const setTop = (key: string, v: Json | undefined) => {
    const pair = root.items.find((p) => keyOf(p) === key);
    if (v === undefined || v === null) {
      if (pair) root.items = root.items.filter((p) => p !== pair);
      return;
    }
    if (pair) pair.value = apply(doc, pair.value, v) as never;
    else root.add(doc.createPair(key, sortKeysDeep(v)));
  };

  setTop('node_type', value.node_type);
  setTop('archetype', value.archetype ?? undefined);
  setTop('properties', value.properties);
  return doc.toString(STRINGIFY);
}

/** Update any YAML file (translation overlay, asset metadata) in place. */
export function mergeYaml(existingText: string, value: JsonObject): string {
  const doc = yaml.parseDocument(existingText);
  if (!isMap(doc.contents)) return renderYaml(sortKeysDeep(value) as JsonObject);
  doc.contents = apply(doc, doc.contents, value) as never;
  return doc.toString(STRINGIFY);
}

// ---------------------------------------------------------------------------
// SKILL.md
// ---------------------------------------------------------------------------

const SKILL_FRONTMATTER = ['name', 'description', 'license', 'metadata'] as const;

/** The SKILL.md a raisin:Skill node stands for — inverse of skillMdToNodeYaml. */
export function nodeToSkillMd(properties: JsonObject): string {
  const fm: JsonObject = {};
  for (const k of SKILL_FRONTMATTER) if (properties[k] !== undefined) fm[k] = properties[k];
  const body = typeof properties.body === 'string' ? properties.body : '';
  return `---\n${yaml.stringify(fm, STRINGIFY)}---\n${body}${body.endsWith('\n') ? '' : '\n'}`;
}
