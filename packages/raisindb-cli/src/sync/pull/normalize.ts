/**
 * Server node → package-file value, and the equality pull decides on.
 *
 * Two things make a naive comparison wrong on every single node, which is why
 * this file exists rather than a `JSON.stringify(a) === JSON.stringify(b)`:
 *
 * 1. The server REWRITES references on write. A package says
 *    `{raisin:ref: /baden-rhinos, raisin:workspace: branding}`; the server
 *    stores `{raisin:ref: <uuid>, raisin:workspace: branding, raisin:path:
 *    /baden-rhinos}`. A uuid means nothing on another installation, so pull
 *    writes references back in PATH form, and compares them by path.
 * 2. A package value may be an `{env:NAME}` token. The server only ever saw
 *    the resolved value — and a live server saw the PRODUCTION value — so a
 *    token is deploy-time configuration owned by the package. It matches any
 *    server value and is never overwritten.
 *
 * Plus the bookkeeping the server adds that a package never declares:
 * `$supertypes` / `$mixins` (computed type membership) and `__*` keys
 * (engine-owned; the write path refuses them, so writing them into a package
 * would make it un-deployable).
 */

import crypto from 'crypto';
import { hasEnvTokens } from '../../env/substitute.js';

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
export type JsonObject = { [k: string]: Json };

const REF = 'raisin:ref';
const REF_WS = 'raisin:workspace';
const REF_PATH = 'raisin:path';

/**
 * Keys a translation overlay never carries. Must match NON_TRANSLATABLE_KEYS
 * in the server's package_install/translation.rs (and operations.ts).
 */
export const NON_TRANSLATABLE_KEYS = new Set([
  'uuid', 'id', 'element_type', 'slug', 'node_type', 'archetype',
  'parent', 'order', 'sort_order', 'weight',
]);

export function isPlainObject(v: unknown): v is JsonObject {
  return !!v && typeof v === 'object' && !Array.isArray(v);
}

function isRefEnvelope(v: unknown): v is JsonObject {
  return isPlainObject(v) && typeof v[REF] === 'string';
}

/** Top-level property keys the server computes or owns. */
export function isServerOwnedKey(key: string): boolean {
  return key.startsWith('$') || key.startsWith('__');
}

/** A reference in the portable form a package carries: path, not uuid. */
function portableRef(ref: JsonObject): JsonObject {
  const out: JsonObject = { ...ref };
  const p = ref[REF_PATH];
  if (typeof p === 'string' && p) out[REF] = p;
  delete out[REF_PATH];
  return out;
}

/** Deep copy with every reference envelope turned into path form. */
export function portable(value: Json): Json {
  if (Array.isArray(value)) return value.map(portable);
  if (isPlainObject(value)) {
    if (isRefEnvelope(value)) return portableRef(value);
    const out: JsonObject = {};
    for (const [k, v] of Object.entries(value as JsonObject)) out[k] = portable(v);
    return out;
  }
  return value;
}

/** Server properties → the properties a package file should declare. */
export function cleanProperties(props: JsonObject | null | undefined): JsonObject {
  const out: JsonObject = {};
  for (const [k, v] of Object.entries(props ?? {})) {
    if (isServerOwnedKey(k)) continue;
    out[k] = portable(v);
  }
  return out;
}

const isAbsent = (v: unknown) => v === undefined || v === null;

/**
 * Does the package value `local` already say what the server value says?
 *
 * Asymmetric on purpose: `local` is what a person wrote (tokens, path refs),
 * `server` is what the server stores (resolved values, uuid refs + path).
 */
export function equalModuloEnv(local: unknown, server: unknown): boolean {
  if (typeof local === 'string' && hasEnvTokens(local)) return true;
  if (isAbsent(local) && isAbsent(server)) return true;

  if (isRefEnvelope(server)) {
    if (!isRefEnvelope(local)) return false;
    const ref = local[REF];
    const sameTarget =
      ref === server[REF] || (typeof server[REF_PATH] === 'string' && ref === server[REF_PATH]);
    if (!sameTarget) return false;
    if ((local[REF_WS] ?? null) !== (server[REF_WS] ?? null)) return false;
    // Any extra key on either side (beyond the three ref keys) must agree.
    const keys = new Set([...Object.keys(local), ...Object.keys(server)]);
    for (const k of keys) {
      if (k === REF || k === REF_WS || k === REF_PATH) continue;
      if (!equalModuloEnv(local[k], server[k])) return false;
    }
    return true;
  }

  if (Array.isArray(local) || Array.isArray(server)) {
    if (!Array.isArray(local) || !Array.isArray(server)) return false;
    if (local.length !== server.length) return false;
    return local.every((v, i) => equalModuloEnv(v, server[i]));
  }

  if (isPlainObject(local) || isPlainObject(server)) {
    if (!isPlainObject(local) || !isPlainObject(server)) return false;
    const keys = new Set([...Object.keys(local), ...Object.keys(server)]);
    for (const k of keys) {
      if (!equalModuloEnv(local[k], server[k])) return false;
    }
    return true;
  }

  return local === server;
}

/** JSON with object keys sorted — the canonical text a hash is taken of. */
export function stableStringify(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(',')}]`;
  if (isPlainObject(value)) {
    const keys = Object.keys(value).sort();
    return `{${keys.map((k) => `${JSON.stringify(k)}:${stableStringify(value[k])}`).join(',')}}`;
  }
  return JSON.stringify(value ?? null);
}

/** Recursively sort object keys (arrays keep their order — it means something). */
export function sortKeysDeep(value: Json): Json {
  if (Array.isArray(value)) return value.map(sortKeysDeep);
  if (isPlainObject(value)) {
    const out: JsonObject = {};
    for (const k of Object.keys(value).sort()) out[k] = sortKeysDeep(value[k]);
    return out;
  }
  return value;
}

export function sha256(data: string | Buffer): string {
  return crypto.createHash('sha256').update(data).digest('hex');
}

export function canonicalHash(value: unknown): string {
  return sha256(stableStringify(value));
}

// ---------------------------------------------------------------------------
// Translation overlays
// ---------------------------------------------------------------------------

function isUuidSection(arr: unknown): arr is JsonObject[] {
  return (
    Array.isArray(arr) &&
    arr.length > 0 &&
    arr.every((item) => isPlainObject(item) && typeof item.uuid === 'string')
  );
}

function sectionDiff(base: JsonObject[], loc: JsonObject[]): JsonObject[] {
  const baseByUuid = new Map(base.map((item) => [item.uuid as string, item]));
  const out: JsonObject[] = [];
  for (const item of loc) {
    const b = baseByUuid.get(item.uuid as string) ?? {};
    const changed: JsonObject = {};
    for (const [field, val] of Object.entries(item)) {
      if (NON_TRANSLATABLE_KEYS.has(field)) continue;
      const bv = b[field];
      if (isUuidSection(val) && isUuidSection(bv)) {
        const nested = sectionDiff(bv, val);
        if (nested.length > 0) changed[field] = nested;
      } else if (stableStringify(bv ?? null) !== stableStringify(val ?? null)) {
        changed[field] = val;
      }
    }
    if (Object.keys(changed).length > 0) out.push({ uuid: item.uuid, ...changed });
  }
  return out;
}

/**
 * The overlay a locale applies on top of the base node, reconstructed from the
 * base properties and the locale-resolved ones — the inverse of the server's
 * `yaml_to_overlay`, in the `.node.{locale}.yaml` shape it reads: top-level
 * fields by name, block fields as `{uuid, field}` entries of their section.
 *
 * Inputs are CLEANED properties (see `cleanProperties`). An empty result means
 * the locale changes nothing, and no translation file is written.
 */
export function computeOverlay(base: JsonObject, localized: JsonObject): JsonObject {
  const out: JsonObject = {};
  for (const [key, val] of Object.entries(localized)) {
    if (NON_TRANSLATABLE_KEYS.has(key)) continue;
    const bv = base[key];
    if (isUuidSection(val) && isUuidSection(bv)) {
      const diff = sectionDiff(bv, val);
      if (diff.length > 0) out[key] = diff;
    } else if (stableStringify(bv ?? null) !== stableStringify(val ?? null)) {
      out[key] = val;
    }
  }
  return out;
}

function collectPointers(prefix: string, arr: JsonObject[], out: Map<string, Json>): void {
  for (const item of arr) {
    const uuid = item.uuid as string;
    for (const [field, val] of Object.entries(item)) {
      if (NON_TRANSLATABLE_KEYS.has(field)) continue;
      if (isUuidSection(val)) collectPointers(`${prefix}/${uuid}/${field}`, val, out);
      else out.set(`${prefix}/${uuid}/${field}`, val);
    }
  }
}

/** An overlay file's meaning as JSON-pointer → value, as the server reads it. */
export function overlayPointers(overlay: JsonObject): Map<string, Json> {
  const out = new Map<string, Json>();
  if (overlay.hidden === true) {
    out.set('#hidden', true);
    return out;
  }
  for (const [key, val] of Object.entries(overlay)) {
    if (NON_TRANSLATABLE_KEYS.has(key)) continue;
    if (isUuidSection(val)) collectPointers(`/${key}`, val, out);
    else out.set(`/${key}`, val);
  }
  return out;
}

/** Do two overlay files mean the same thing to the server? */
export function overlaysEqual(local: JsonObject, server: JsonObject): boolean {
  const a = overlayPointers(local);
  const b = overlayPointers(server);
  if (a.size !== b.size) return false;
  for (const [k, v] of a) {
    if (!b.has(k) || !equalModuloEnv(v, b.get(k))) return false;
  }
  return true;
}
