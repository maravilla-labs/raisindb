#!/usr/bin/env node
/**
 * Build the builtin type catalog shipped inside the CLI (dist/builtin-types.json).
 *
 * Walks RaisinDB's own types — crates/raisin-core/global_nodetypes/*.yaml and
 * builtin-packages/<pkg>/{nodetypes,archetypes,elementtypes}/*.yaml — and
 * reduces them to type catalog format v1 (see src/wasm/type-catalog.ts), with
 * `source: "builtin"`. The CLI's translation validator loads it offline next
 * to any downloaded catalogs in ~/.raisindb/catalogs/.
 *
 * Usage: node scripts/build-builtin-catalog.mjs [--out <file>] [--repo <root>]
 *
 * The version recorded is, in order: $RAISINDB_VERSION, the newest v* git tag
 * of the repository, the CLI package version.
 */

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import yaml from 'yaml';

const here = path.dirname(fileURLToPath(import.meta.url));
const cliDir = path.resolve(here, '..');

/** FieldDef reduced to { $type, name, translatable?, fields? } — recursively. */
export function reduceField(field) {
  if (!field || typeof field !== 'object' || typeof field.name !== 'string') return null;
  const out = { $type: typeof field.$type === 'string' ? field.$type : 'Unknown', name: field.name };
  if (field.translatable === true) out.translatable = true;
  if (Array.isArray(field.fields)) {
    const nested = field.fields.map(reduceField).filter(Boolean);
    out.fields = nested;
  }
  return out;
}

export function reduceArchetype(doc) {
  const out = { name: doc.name };
  if (typeof doc.base_node_type === 'string') out.base_node_type = doc.base_node_type;
  if (typeof doc.extends === 'string') out.extends = doc.extends;
  out.fields = Array.isArray(doc.fields) ? doc.fields.map(reduceField).filter(Boolean) : [];
  return out;
}

export function reduceElementType(doc) {
  const out = { name: doc.name };
  if (typeof doc.extends === 'string') out.extends = doc.extends;
  out.fields = Array.isArray(doc.fields) ? doc.fields.map(reduceField).filter(Boolean) : [];
  return out;
}

export function reduceNodeType(doc) {
  const out = { name: doc.name };
  if (typeof doc.extends === 'string') out.extends = doc.extends;
  let props = doc.properties;
  // Tolerate the map form { name: { type, ... } } as well as the list form.
  if (props && typeof props === 'object' && !Array.isArray(props)) {
    props = Object.entries(props).map(([name, def]) => ({ ...(def ?? {}), name }));
  }
  out.properties = Array.isArray(props)
    ? props
        .filter(p => p && typeof p === 'object' && typeof p.name === 'string')
        .map(p => {
          const r = { name: p.name };
          if (p.is_translatable === true) r.is_translatable = true;
          if (typeof p.type === 'string') r.type = p.type;
          return r;
        })
    : [];
  return out;
}

function yamlFiles(dir) {
  try {
    return fs
      .readdirSync(dir)
      .filter(f => /\.ya?ml$/i.test(f))
      .sort()
      .map(f => path.join(dir, f));
  } catch {
    return [];
  }
}

function readDoc(file) {
  const doc = yaml.parse(fs.readFileSync(file, 'utf-8'));
  return doc && typeof doc === 'object' && typeof doc.name === 'string' ? doc : null;
}

/**
 * Build a v1 catalog from directories of node type, archetype and element type
 * YAML files. Later definitions of the same name replace earlier ones.
 */
export function buildCatalog({ nodeTypeDirs = [], archetypeDirs = [], elementTypeDirs = [], source, packageVersion }) {
  const collect = (dirs, reduce) => {
    const byName = new Map();
    for (const dir of dirs) {
      for (const file of yamlFiles(dir)) {
        const doc = readDoc(file);
        if (doc) byName.set(doc.name, reduce(doc));
      }
    }
    return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
  };
  return {
    format: 'raisin-type-catalog',
    version: 1,
    source,
    package_version: packageVersion,
    generated_at: new Date().toISOString(),
    archetypes: collect(archetypeDirs, reduceArchetype),
    elementTypes: collect(elementTypeDirs, reduceElementType),
    nodeTypes: collect(nodeTypeDirs, reduceNodeType),
  };
}

function resolveVersion(repoRoot) {
  if (process.env.RAISINDB_VERSION) return process.env.RAISINDB_VERSION.replace(/^v/, '');
  try {
    const tag = execFileSync('git', ['-C', repoRoot, 'describe', '--tags', '--abbrev=0', '--match', 'v[0-9]*'], {
      encoding: 'utf-8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
    if (tag) return tag.replace(/^v/, '');
  } catch {
    // Not a git checkout (e.g. a source tarball) — fall through.
  }
  return JSON.parse(fs.readFileSync(path.join(cliDir, 'package.json'), 'utf-8')).version;
}

export function buildBuiltinCatalog(repoRoot) {
  const pkgRoot = path.join(repoRoot, 'builtin-packages');
  let packages = [];
  try {
    packages = fs
      .readdirSync(pkgRoot, { withFileTypes: true })
      .filter(d => d.isDirectory())
      .map(d => path.join(pkgRoot, d.name))
      .sort();
  } catch {
    // No builtin-packages directory.
  }
  return buildCatalog({
    nodeTypeDirs: [path.join(repoRoot, 'crates', 'raisin-core', 'global_nodetypes'), ...packages.map(p => path.join(p, 'nodetypes'))],
    archetypeDirs: packages.map(p => path.join(p, 'archetypes')),
    elementTypeDirs: packages.map(p => path.join(p, 'elementtypes')),
    source: 'builtin',
    packageVersion: resolveVersion(repoRoot),
  });
}

function main(argv) {
  const arg = name => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const repoRoot = path.resolve(arg('--repo') ?? path.join(cliDir, '..', '..'));
  const out = path.resolve(arg('--out') ?? path.join(cliDir, 'dist', 'builtin-types.json'));
  const catalog = buildBuiltinCatalog(repoRoot);
  if (catalog.nodeTypes.length === 0) {
    throw new Error(`No builtin node types found under ${repoRoot} — is --repo right?`);
  }
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, JSON.stringify(catalog) + '\n');
  console.log(
    `Builtin type catalog ${catalog.package_version}: ${catalog.nodeTypes.length} node types, ` +
      `${catalog.archetypes.length} archetypes, ${catalog.elementTypes.length} element types -> ${path.relative(process.cwd(), out)}`,
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
