/**
 * Type catalogs: schemas of types that live OUTSIDE the package being validated.
 *
 * A package may extend or name types it does not ship itself — an archetype
 * `extends: standard:ContentPage`, a node `archetype: standard:ArticlePage`, a
 * block of `element_type: standard:Hero`. Without their definitions the
 * translation validator cannot tell an inherited translatable field from a
 * stray key, so it skips them. A type catalog supplies those definitions,
 * reduced to what validation needs.
 *
 * Two sources are loaded, both offline — the CLI never fetches a catalog:
 *
 *  - builtin: RaisinDB's own types (raisin:* global node types and the
 *    builtin-packages), generated at CLI build time into
 *    `dist/builtin-types.json` by scripts/build-builtin-catalog.mjs.
 *  - every `~/.raisindb/catalogs/*.json`, e.g. the Studio catalog that
 *    `maravilla update` (or a build server) downloads next to the Studio
 *    package. When several files carry the same `source`, the highest
 *    `package_version` wins.
 *
 * Catalog format v1:
 *
 *   { "format": "raisin-type-catalog", "version": 1, "source": "studio",
 *     "package_version": "0.3.10", "generated_at": "<ISO>",
 *     "archetypes":   [{ name, base_node_type?, extends?, fields: FieldDef[] }],
 *     "elementTypes": [{ name, extends?, fields: FieldDef[] }],
 *     "nodeTypes":    [{ name, extends?, properties: [{ name, is_translatable?, type? }] }] }
 *
 *   FieldDef = { "$type", name, translatable?, fields?: FieldDef[] }
 *
 * The Studio assets host answers a missing file with its SPA's index.html and
 * status 200, so a downloaded "catalog" can be HTML: every file is checked for
 * JSON, `format` and `version` before it is used, and ignored otherwise.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { fileURLToPath } from 'url';

export const TYPE_CATALOG_FORMAT = 'raisin-type-catalog';
export const TYPE_CATALOG_VERSION = 1;

export interface CatalogFieldDef {
  $type: string;
  name: string;
  translatable?: boolean;
  fields?: CatalogFieldDef[];
}

export interface CatalogArchetype {
  name: string;
  base_node_type?: string;
  extends?: string;
  fields: CatalogFieldDef[];
}

export interface CatalogElementType {
  name: string;
  extends?: string;
  fields: CatalogFieldDef[];
}

export interface CatalogNodeTypeProp {
  name: string;
  is_translatable?: boolean;
  type?: string;
}

export interface CatalogNodeType {
  name: string;
  extends?: string;
  properties: CatalogNodeTypeProp[];
}

export interface TypeCatalog {
  format: typeof TYPE_CATALOG_FORMAT;
  version: typeof TYPE_CATALOG_VERSION;
  source: string;
  package_version: string;
  generated_at?: string;
  archetypes: CatalogArchetype[];
  elementTypes: CatalogElementType[];
  nodeTypes: CatalogNodeType[];
}

/** Definitions from all loaded catalogs, keyed by type name. */
export interface ExternalSchemas {
  archetypes: Map<string, CatalogArchetype>;
  elementTypes: Map<string, CatalogElementType>;
  nodeTypes: Map<string, CatalogNodeType>;
}

export interface LoadedTypeCatalogs {
  /** The catalogs in use, one per source. */
  catalogs: Array<{ source: string; package_version: string; file: string }>;
  /** Files that were found but are not a usable v1 catalog. */
  ignored: Array<{ file: string; reason: string }>;
  schemas: ExternalSchemas;
}

export function emptyExternalSchemas(): ExternalSchemas {
  return { archetypes: new Map(), elementTypes: new Map(), nodeTypes: new Map() };
}

/** `~/.raisindb/catalogs` — where downloaded catalogs are kept. */
export function defaultCatalogDir(): string {
  return path.join(os.homedir(), '.raisindb', 'catalogs');
}

/**
 * The builtin catalog shipped with the CLI. From `dist/wasm/type-catalog.js`
 * it is `dist/builtin-types.json`; when running from source (tsx, vitest) the
 * built copy in `dist/` is used if there is one.
 */
export function defaultBuiltinCatalogPaths(): string[] {
  const here = path.dirname(fileURLToPath(import.meta.url));
  return [
    path.resolve(here, '..', 'builtin-types.json'),
    path.resolve(here, '..', '..', 'dist', 'builtin-types.json'),
  ];
}

/**
 * Parse and check one catalog. Returns the catalog, or the reason it cannot
 * be used.
 */
export function parseTypeCatalog(text: string): TypeCatalog | { error: string } {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return { error: 'not JSON (a missing file on the assets host is served as HTML)' };
  }
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    return { error: 'not a JSON object' };
  }
  const c = data as Record<string, unknown>;
  if (c.format !== TYPE_CATALOG_FORMAT) {
    return { error: `format is ${JSON.stringify(c.format)}, expected "${TYPE_CATALOG_FORMAT}"` };
  }
  if (c.version !== TYPE_CATALOG_VERSION) {
    return { error: `unsupported catalog version ${JSON.stringify(c.version)}` };
  }
  if (typeof c.source !== 'string' || !c.source) {
    return { error: 'missing "source"' };
  }
  for (const key of ['archetypes', 'elementTypes', 'nodeTypes']) {
    if (c[key] !== undefined && !Array.isArray(c[key])) {
      return { error: `"${key}" is not an array` };
    }
  }
  const named = <T>(v: unknown): T[] =>
    Array.isArray(v)
      ? (v.filter(x => x && typeof x === 'object' && typeof (x as { name?: unknown }).name === 'string') as T[])
      : [];
  return {
    format: TYPE_CATALOG_FORMAT,
    version: TYPE_CATALOG_VERSION,
    source: c.source,
    package_version: typeof c.package_version === 'string' ? c.package_version : '',
    generated_at: typeof c.generated_at === 'string' ? c.generated_at : undefined,
    archetypes: named<CatalogArchetype>(c.archetypes).map(a => ({
      ...a,
      fields: Array.isArray(a.fields) ? a.fields : [],
    })),
    elementTypes: named<CatalogElementType>(c.elementTypes).map(e => ({
      ...e,
      fields: Array.isArray(e.fields) ? e.fields : [],
    })),
    nodeTypes: named<CatalogNodeType>(c.nodeTypes).map(n => ({
      ...n,
      properties: Array.isArray(n.properties) ? n.properties : [],
    })),
  };
}

/** Compare dotted numeric versions ("0.3.10" > "0.3.9"); non-numeric parts compare as text. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(/[.+-]/);
  const pb = b.split(/[.+-]/);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] ?? '';
    const y = pb[i] ?? '';
    const nx = Number(x);
    const ny = Number(y);
    if (x !== '' && y !== '' && !Number.isNaN(nx) && !Number.isNaN(ny)) {
      if (nx !== ny) return nx - ny;
    } else if (x !== y) {
      return x < y ? -1 : 1;
    }
  }
  return 0;
}

export interface LoadTypeCatalogsOptions {
  /** Directory of downloaded catalogs. Default: ~/.raisindb/catalogs */
  catalogDir?: string;
  /** Candidate paths of the builtin catalog; the first that exists is used. */
  builtinPaths?: string[];
}

/**
 * Load the builtin catalog and every catalog in the catalog directory.
 * Never throws: an unreadable directory means no catalogs.
 */
export function loadTypeCatalogs(options: LoadTypeCatalogsOptions = {}): LoadedTypeCatalogs {
  const catalogDir = options.catalogDir ?? defaultCatalogDir();
  const builtinPaths = options.builtinPaths ?? defaultBuiltinCatalogPaths();
  const ignored: LoadedTypeCatalogs['ignored'] = [];
  const bySource = new Map<string, { catalog: TypeCatalog; file: string }>();

  const consider = (file: string) => {
    let text: string;
    try {
      text = fs.readFileSync(file, 'utf-8');
    } catch (e) {
      ignored.push({ file, reason: e instanceof Error ? e.message : String(e) });
      return;
    }
    const parsed = parseTypeCatalog(text);
    if ('error' in parsed) {
      ignored.push({ file, reason: parsed.error });
      return;
    }
    const prev = bySource.get(parsed.source);
    if (!prev || compareVersions(parsed.package_version, prev.catalog.package_version) > 0) {
      bySource.set(parsed.source, { catalog: parsed, file });
    }
  };

  const builtin = builtinPaths.find(p => {
    try {
      return fs.statSync(p).isFile();
    } catch {
      return false;
    }
  });
  if (builtin) consider(builtin);

  let entries: string[] = [];
  try {
    entries = fs.readdirSync(catalogDir).filter(f => f.endsWith('.json')).sort();
  } catch {
    // No catalog directory — no downloaded catalogs.
  }
  for (const f of entries) consider(path.join(catalogDir, f));

  // Builtin first, so a downloaded catalog wins on a (unlikely) name clash.
  const ordered = [...bySource.values()].sort((a, b) =>
    a.catalog.source === 'builtin' ? -1 : b.catalog.source === 'builtin' ? 1 : a.catalog.source.localeCompare(b.catalog.source),
  );
  const schemas = emptyExternalSchemas();
  for (const { catalog } of ordered) {
    for (const a of catalog.archetypes) schemas.archetypes.set(a.name, a);
    for (const e of catalog.elementTypes) schemas.elementTypes.set(e.name, e);
    for (const n of catalog.nodeTypes) schemas.nodeTypes.set(n.name, n);
  }

  return {
    catalogs: ordered.map(({ catalog, file }) => ({
      source: catalog.source,
      package_version: catalog.package_version,
      file,
    })),
    ignored,
    schemas,
  };
}

/**
 * One info line naming the catalogs in use, e.g.
 * "Using type catalogs: studio 0.3.10, builtin 0.6.41".
 */
export function describeTypeCatalogs(loaded: LoadedTypeCatalogs): string {
  const label = (c: { source: string; package_version: string }) =>
    c.package_version ? `${c.source} ${c.package_version}` : c.source;
  const studio = loaded.catalogs.filter(c => c.source === 'studio');
  const others = loaded.catalogs.filter(c => c.source !== 'studio' && c.source !== 'builtin');
  const builtin = loaded.catalogs.filter(c => c.source === 'builtin');
  const ignoredNote =
    loaded.ignored.length > 0
      ? ` (ignored ${loaded.ignored.length} invalid catalog file(s): ${loaded.ignored
          .map(i => `${path.basename(i.file)}: ${i.reason}`)
          .join('; ')})`
      : '';
  if (studio.length === 0) {
    const rest = [...others, ...builtin].map(label);
    const suffix = rest.length > 0 ? ` (using ${rest.join(', ')})` : '';
    return `No Studio type catalog — inherited fields not checked; run maravilla update${suffix}${ignoredNote}`;
  }
  return `Using type catalogs: ${[...studio, ...others, ...builtin].map(label).join(', ')}${ignoredNote}`;
}
