/**
 * Translation file validation for RaisinDB packages.
 *
 * Translation files (e.g., .node.de.yaml) are filtered out of the WASM
 * validator pipeline and validated here in TypeScript instead, because
 * the WASM content validator requires a `node_type` field that translations
 * intentionally omit.
 *
 * Schema-aware: checks that every translated key is marked `translatable`
 * in the corresponding archetype, element type, or node type definition.
 */

import * as path from 'path';
import yaml from 'yaml';
import { parseTranslationLocale } from '../sync/operations.js';
import type { ValidationResult, ValidationError } from './types.js';
import { ErrorCodes } from './types.js';
import type { ExternalSchemas } from './type-catalog.js';

// ---------------------------------------------------------------------------
// Schema types (parsed from package YAML files)
// ---------------------------------------------------------------------------

interface FieldDef {
  $type: string;
  name: string;
  translatable?: boolean;
  fields?: FieldDef[];
}

interface ArchetypeSchema {
  name: string;
  base_node_type?: string;
  extends?: string;
  fields: FieldDef[];
}

interface ElementTypeSchema {
  name: string;
  extends?: string;
  fields: FieldDef[];
}

/**
 * A schema with its `extends` chain resolved against the package and the
 * external type catalogs.
 *
 * `fields` holds the merged fields of every ancestor found (a child's field
 * overrides a parent's field of the same name). `open` is true when the chain
 * reaches a parent that is neither in the package nor in a loaded catalog
 * (e.g. `standard:ContentPage` with no Studio catalog installed): fields
 * inherited from it are invisible here, so a translated key that is not
 * declared anywhere in the known part of the chain must not be reported.
 */
interface ResolvedSchema {
  name: string;
  fields: FieldDef[];
  open: boolean;
}

interface NodeTypeProp {
  name: string;
  is_translatable?: boolean;
}

interface NodeTypeSchema {
  name: string;
  properties: NodeTypeProp[];
}

/**
 * Schema context built from all package files.
 *
 * `external` holds types from the loaded type catalogs (builtin RaisinDB
 * types, the Studio catalog). Package definitions always win over them.
 */
export interface SchemaContext {
  archetypes: Map<string, ArchetypeSchema>;
  elementTypes: Map<string, ElementTypeSchema>;
  nodeTypes: Map<string, NodeTypeSchema>;
  external?: ExternalSchemas;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/**
 * Structural / metadata keys that must not appear in translation files.
 * Mirrors the canonical `NON_TRANSLATABLE_KEYS` constant in
 * crates/raisin-validation/src/field_helpers.rs — keep in sync.
 */
export const NON_TRANSLATABLE_KEYS: ReadonlySet<string> = new Set([
  'uuid',
  'id',
  'element_type',
  'slug',
  'node_type',
  'archetype',
  'parent',
  'order',
  'sort_order',
  'weight',
]);

/**
 * Subset of NON_TRANSLATABLE_KEYS that serve as identity keys within
 * section/composite items. These are silently skipped (no warning)
 * when found in translation items, since they're needed for matching.
 */
const SECTION_IDENTIFIER_KEYS: ReadonlySet<string> = new Set(['uuid']);

// ---------------------------------------------------------------------------
// Schema helpers
// ---------------------------------------------------------------------------

/**
 * Build a SchemaContext by parsing archetype, element type, and node type
 * YAML files from the package file map.
 */
export function buildSchemaContext(
  allFiles: Record<string, string>,
  external?: ExternalSchemas,
): SchemaContext {
  const archetypes = new Map<string, ArchetypeSchema>();
  const elementTypes = new Map<string, ElementTypeSchema>();
  const nodeTypes = new Map<string, NodeTypeSchema>();

  for (const [filePath, content] of Object.entries(allFiles)) {
    try {
      const parsed = yaml.parse(content);
      if (!parsed || typeof parsed !== 'object') continue;

      const hasFieldsOrParent =
        Array.isArray(parsed.fields) || typeof parsed.extends === 'string';
      if (filePath.startsWith('archetypes/') && parsed.name && hasFieldsOrParent) {
        archetypes.set(parsed.name, {
          ...parsed,
          fields: Array.isArray(parsed.fields) ? parsed.fields : [],
        } as ArchetypeSchema);
      } else if (filePath.startsWith('elementtypes/') && parsed.name && hasFieldsOrParent) {
        elementTypes.set(parsed.name, {
          ...parsed,
          fields: Array.isArray(parsed.fields) ? parsed.fields : [],
        } as ElementTypeSchema);
      } else if (filePath.startsWith('nodetypes/') && parsed.name && parsed.properties) {
        nodeTypes.set(parsed.name, parsed as NodeTypeSchema);
      }
    } catch {
      // Skip unparseable files — they'll be caught by the WASM validator
    }
  }

  return { archetypes, elementTypes, nodeTypes, external };
}

/** Look up an archetype in the package, then in the external catalogs. */
function findArchetype(ctx: SchemaContext, name: string): ArchetypeSchema | undefined {
  return ctx.archetypes.get(name) ?? ctx.external?.archetypes.get(name);
}

/** Look up an element type in the package, then in the external catalogs. */
function findElementType(ctx: SchemaContext, name: string): ElementTypeSchema | undefined {
  return ctx.elementTypes.get(name) ?? ctx.external?.elementTypes.get(name);
}

/**
 * Collect the set of directly-translatable field names from a list of FieldDefs.
 *
 * Nested CompositeField sub-fields are NOT included here — their translatable
 * descendants are validated by recursing into the nested array (see
 * `checkCompositeItems`).
 */
function collectTranslatableFields(fields: FieldDef[]): Set<string> {
  const result = new Set<string>();
  for (const f of fields) {
    if (f.translatable === true) {
      result.add(f.name);
    }
  }
  return result;
}

/**
 * Whether a field — or any nested CompositeField descendant — is translatable.
 * A repeatable composite needs item UUIDs whenever this is true for any of its
 * sub-fields, because the translation pointer (`/field/<uuid>/…`) must be
 * anchorable through this level even when only a nested composite is
 * translatable.
 */
function fieldHasTranslatableDescendant(f: FieldDef): boolean {
  if (f.translatable === true) return true;
  if (f.$type === 'CompositeField' && Array.isArray(f.fields)) {
    return f.fields.some(fieldHasTranslatableDescendant);
  }
  return false;
}

/**
 * Resolve a schema's `extends` chain against the schemas known in the package
 * and the external catalogs (`lookup`). Parent fields come first; a child
 * field overrides a parent field by name.
 */
function resolveChain<T extends { name: string; extends?: string; fields: FieldDef[] }>(
  schema: T,
  lookup: (name: string) => T | undefined,
): ResolvedSchema {
  const chain: T[] = [];
  const seen = new Set<string>();
  let open = false;
  let current: T | undefined = schema;
  while (current) {
    if (seen.has(current.name)) break; // cycle guard
    seen.add(current.name);
    chain.push(current);
    const parentName: string | undefined = current.extends;
    if (!parentName) break;
    const parent = lookup(parentName);
    if (!parent) {
      open = true;
      break;
    }
    current = parent;
  }

  const byName = new Map<string, FieldDef>();
  for (let i = chain.length - 1; i >= 0; i--) {
    for (const f of chain[i].fields ?? []) {
      if (f && typeof f.name === 'string') byName.set(f.name, f);
    }
  }
  return { name: schema.name, fields: [...byName.values()], open };
}

/**
 * Find the archetype that governs a node.
 *
 * An explicitly named archetype is looked up in the package, then in the type
 * catalogs (e.g. `standard:ArticlePage` from the Studio catalog). One found in
 * neither is an unknown schema: return undefined so no schema checks run.
 * Only when the node names no archetype do we fall back to a LOCAL archetype
 * whose `base_node_type` matches the node's `node_type` — never to a catalog
 * archetype, since many of those share one base node type.
 */
function resolveArchetype(
  nodeType: string | undefined,
  archetypeName: string | undefined,
  ctx: SchemaContext,
): ResolvedSchema | undefined {
  const lookup = (name: string) => findArchetype(ctx, name);
  if (archetypeName) {
    const a = lookup(archetypeName);
    return a ? resolveChain(a, lookup) : undefined;
  }
  if (nodeType) {
    for (const a of ctx.archetypes.values()) {
      if (a.base_node_type === nodeType) return resolveChain(a, lookup);
    }
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Check whether a relative package path is a translation file.
 * Must be under `content/` and have a parseable locale suffix.
 */
export function isTranslationFile(relativePath: string): boolean {
  if (!relativePath.startsWith('content/')) return false;
  const basename = path.basename(relativePath);
  return parseTranslationLocale(basename) !== null;
}

/**
 * Derive the base node path from a translation path.
 *
 * `.node.de.yaml`  -> `.node.yaml`
 * `about.de.yaml`  -> `about.yaml`
 */
export function getBaseNodePath(translationPath: string): string {
  const dir = path.dirname(translationPath);
  const basename = path.basename(translationPath);

  if (basename.startsWith('.node.')) {
    return path.join(dir, '.node.yaml');
  }

  const withoutYaml = basename.slice(0, -'.yaml'.length);
  const dotPos = withoutYaml.lastIndexOf('.');
  const baseName = withoutYaml.slice(0, dotPos);
  return path.join(dir, `${baseName}.yaml`);
}

/**
 * Validate a single translation file against the package schema.
 */
export function validateTranslationFile(
  filePath: string,
  content: string,
  allFiles: Record<string, string>,
  ctx: SchemaContext,
): ValidationResult {
  const errors: ValidationError[] = [];
  const warnings: ValidationError[] = [];

  // 1. YAML parse
  let parsed: unknown;
  try {
    parsed = yaml.parse(content);
  } catch (e) {
    errors.push({
      file_path: filePath,
      field_path: '',
      error_code: ErrorCodes.TRANSLATION_INVALID_YAML,
      message: e instanceof Error ? e.message : 'Invalid YAML',
      severity: 'error',
      fix_type: 'manual',
    });
    return { success: false, file_type: 'translation', errors, warnings };
  }

  // 2. Must be a plain object
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    errors.push({
      file_path: filePath,
      field_path: '',
      error_code: ErrorCodes.TRANSLATION_NOT_OBJECT,
      message: 'Translation file must be a YAML mapping',
      severity: 'error',
      fix_type: 'manual',
    });
    return { success: false, file_type: 'translation', errors, warnings };
  }

  const obj = parsed as Record<string, unknown>;

  // 3. Hidden shortcut — { hidden: true } is a valid tombstone
  if (obj.hidden === true && Object.keys(obj).length === 1) {
    return { success: true, file_type: 'translation', errors, warnings };
  }

  // 4. Base node exists
  const basePath = getBaseNodePath(filePath);
  const baseContent = allFiles[basePath];
  if (!baseContent) {
    warnings.push({
      file_path: filePath,
      field_path: '',
      error_code: ErrorCodes.TRANSLATION_MISSING_BASE_NODE,
      message: `Base node file not found: ${basePath}`,
      severity: 'warning',
      fix_type: 'manual',
    });
  }

  // 5. NON_TRANSLATABLE_KEYS check (always applies)
  checkNonTranslatableKeys(obj, '', filePath, warnings);
  for (const [key, value] of Object.entries(obj)) {
    if (Array.isArray(value)) {
      for (let i = 0; i < value.length; i++) {
        const item = value[i];
        if (item && typeof item === 'object' && !Array.isArray(item) && 'uuid' in item) {
          checkNonTranslatableKeys(
            item as Record<string, unknown>,
            `${key}[${i}]`,
            filePath,
            warnings,
          );
        }
      }
    }
  }

  // 6. Schema-aware translatability check
  if (baseContent) {
    checkTranslatability(obj, filePath, baseContent, ctx, errors);
  }

  return {
    success: errors.length === 0,
    file_type: 'translation',
    errors,
    warnings,
  };
}

/**
 * Check translated keys against the schema's `translatable` markers.
 * Produces errors for fields that exist in the schema but are NOT translatable.
 */
function checkTranslatability(
  translationObj: Record<string, unknown>,
  filePath: string,
  baseContent: string,
  ctx: SchemaContext,
  errors: ValidationError[],
): void {
  let baseNode: Record<string, unknown>;
  try {
    const p = yaml.parse(baseContent);
    if (!p || typeof p !== 'object' || Array.isArray(p)) return;
    baseNode = p as Record<string, unknown>;
  } catch {
    return; // base node unparseable — already caught elsewhere
  }

  const nodeType = baseNode.node_type as string | undefined;
  const archetypeName = baseNode.archetype as string | undefined;

  const archetype = resolveArchetype(nodeType, archetypeName, ctx);
  if (!archetype) {
    // No schema found (e.g., built-in types like raisin:Folder).
    // Fall back to NON_TRANSLATABLE_KEYS only — no schema errors.
    return;
  }

  const topTranslatable = collectTranslatableFields(archetype.fields);

  // Check top-level keys
  for (const key of Object.keys(translationObj)) {
    if (NON_TRANSLATABLE_KEYS.has(key)) continue; // already warned
    if (key === 'hidden') continue;

    // Check if this key maps to a SectionField or CompositeField (arrays)
    const fieldDef = archetype.fields.find(f => f.name === key);

    if (fieldDef && fieldDef.$type === 'SectionField') {
      // Section fields contain elements — check element items
      const arr = translationObj[key];
      if (Array.isArray(arr)) {
        // Extract the corresponding section array from the base node's properties
        const baseProps = baseNode.properties as Record<string, unknown> | undefined;
        const baseSectionItems = baseProps && Array.isArray(baseProps[key]) ? baseProps[key] as unknown[] : [];
        checkSectionItems(arr, key, filePath, ctx, errors, baseSectionItems);
      }
      continue;
    }

    if (fieldDef && fieldDef.$type === 'CompositeField') {
      // Composite fields contain repeatable sub-objects
      const arr = translationObj[key];
      if (Array.isArray(arr)) {
        checkCompositeItems(arr, key, filePath, fieldDef, errors);
      }
      continue;
    }

    // An embedded element's own fields are not described here — don't
    // report the embedding field itself.
    if (fieldDef && fieldDef.$type === 'ElementField' && isPlainObject(translationObj[key])) continue;

    // Unknown key on a schema whose chain reaches a parent we cannot see:
    // it may be an inherited field — don't report it.
    if (!fieldDef && archetype.open) continue;

    // Scalar field — must be in the translatable set
    if (!topTranslatable.has(key)) {
      errors.push({
        file_path: filePath,
        field_path: key,
        error_code: ErrorCodes.TRANSLATION_FIELD_NOT_TRANSLATABLE,
        message: `Field '${key}' is not marked as translatable in archetype '${archetype.name}'`,
        severity: 'error',
        fix_type: 'manual',
      });
    }
  }
}

/**
 * Check element items inside a SectionField array.
 */
function checkSectionItems(
  items: unknown[],
  sectionKey: string,
  filePath: string,
  ctx: SchemaContext,
  errors: ValidationError[],
  baseItems: unknown[],
): void {
  // Build uuid → base element map from base items
  const uuidToBase = new Map<string, Record<string, unknown>>();
  for (const baseItem of baseItems) {
    if (baseItem && typeof baseItem === 'object' && !Array.isArray(baseItem)) {
      const rec = baseItem as Record<string, unknown>;
      if (typeof rec.uuid === 'string' && typeof rec.element_type === 'string') {
        uuidToBase.set(rec.uuid, rec);
      }
    }
  }

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (!item || typeof item !== 'object' || Array.isArray(item)) continue;
    const rec = item as Record<string, unknown>;

    // Resolve element_type from base items via uuid
    const uuid = rec.uuid as string | undefined;
    const baseElement = uuid ? uuidToBase.get(uuid) : undefined;
    const elementTypeName = baseElement?.element_type as string | undefined;
    if (!baseElement || !elementTypeName) continue;

    const et = findElementType(ctx, elementTypeName);
    if (!et) continue; // unknown element type — warned elsewhere
    const etSchema = resolveChain(et, name => findElementType(ctx, name));

    const etTranslatable = collectTranslatableFields(etSchema.fields);

    for (const key of Object.keys(rec)) {
      if (NON_TRANSLATABLE_KEYS.has(key)) continue;
      if (SECTION_IDENTIFIER_KEYS.has(key)) continue;

      const etFieldDef = etSchema.fields.find(f => f.name === key);

      // A container element (standard:Section, standard:Variants, …) holds
      // its own SectionField: check the nested elements, not the field.
      if (etFieldDef && etFieldDef.$type === 'SectionField') {
        const subArr = rec[key];
        if (Array.isArray(subArr)) {
          const baseSub = Array.isArray(baseElement[key]) ? (baseElement[key] as unknown[]) : [];
          checkSectionItems(subArr, `${sectionKey}[${i}].${key}`, filePath, ctx, errors, baseSub);
        }
        continue;
      }

      // An embedded element's own fields are not described here — don't
      // report the embedding field itself.
      if (etFieldDef && etFieldDef.$type === 'ElementField' && isPlainObject(rec[key])) continue;

      // Check CompositeField sub-arrays inside elements
      if (etFieldDef && etFieldDef.$type === 'CompositeField') {
        const subArr = rec[key];
        if (Array.isArray(subArr)) {
          checkCompositeItems(
            subArr,
            `${sectionKey}[${i}].${key}`,
            filePath,
            etFieldDef,
            errors,
          );
        }
        continue;
      }

      // Possibly inherited from a parent element type outside the package.
      if (!etFieldDef && etSchema.open) continue;

      if (!etTranslatable.has(key)) {
        errors.push({
          file_path: filePath,
          field_path: `${sectionKey}[${i}].${key}`,
          error_code: ErrorCodes.TRANSLATION_FIELD_NOT_TRANSLATABLE,
          message: `Field '${key}' is not marked as translatable in element type '${elementTypeName}'`,
          severity: 'error',
          fix_type: 'manual',
        });
      }
    }
  }
}

/**
 * Check items inside a CompositeField array against sub-field translatability.
 *
 * When the composite has translatable sub-fields, each item must have a unique
 * `uuid` for per-field translation overlay merging. Without UUIDs, the entire
 * array would be replaced on translation, losing non-translatable fields.
 */
function checkCompositeItems(
  items: unknown[],
  parentPath: string,
  filePath: string,
  compositeDef: FieldDef,
  errors: ValidationError[],
): void {
  if (!compositeDef.fields) return;
  const subFields = compositeDef.fields;
  const subTranslatable = collectTranslatableFields(subFields);
  // Items need UUIDs if this composite has translatable content anywhere in its
  // sub-tree — directly OR inside a nested composite — so the deep pointer can
  // be anchored through this level.
  const requiresUuid = subFields.some(fieldHasTranslatableDescendant);

  if (requiresUuid) {
    const seenUuids = new Set<string>();
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (!item || typeof item !== 'object' || Array.isArray(item)) continue;
      const rec = item as Record<string, unknown>;

      if (!rec.uuid || typeof rec.uuid !== 'string') {
        errors.push({
          file_path: filePath,
          field_path: `${parentPath}[${i}]`,
          error_code: ErrorCodes.COMPOSITE_MISSING_UUID,
          message: `Item ${parentPath}[${i}] requires a 'uuid' field because the composite has translatable sub-fields`,
          severity: 'error',
          fix_type: 'manual',
        });
      } else if (seenUuids.has(rec.uuid)) {
        errors.push({
          file_path: filePath,
          field_path: `${parentPath}[${i}].uuid`,
          error_code: ErrorCodes.COMPOSITE_DUPLICATE_UUID,
          message: `Duplicate uuid '${rec.uuid}' in composite at ${parentPath}[${i}]`,
          severity: 'error',
          fix_type: 'manual',
        });
      } else {
        seenUuids.add(rec.uuid);
      }
    }
  }

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (!item || typeof item !== 'object' || Array.isArray(item)) continue;
    const rec = item as Record<string, unknown>;

    for (const key of Object.keys(rec)) {
      if (NON_TRANSLATABLE_KEYS.has(key)) continue;
      if (SECTION_IDENTIFIER_KEYS.has(key)) continue;

      // Nested CompositeField: recurse into its items rather than flagging the
      // field itself as non-translatable.
      const subDef = subFields.find(f => f.name === key);
      if (subDef && subDef.$type === 'CompositeField') {
        const subArr = rec[key];
        if (Array.isArray(subArr)) {
          checkCompositeItems(
            subArr,
            `${parentPath}[${i}].${key}`,
            filePath,
            subDef,
            errors,
          );
        }
        continue;
      }

      if (!subTranslatable.has(key)) {
        errors.push({
          file_path: filePath,
          field_path: `${parentPath}[${i}].${key}`,
          error_code: ErrorCodes.TRANSLATION_FIELD_NOT_TRANSLATABLE,
          message: `Field '${key}' is not marked as translatable in composite field '${compositeDef.name}'`,
          severity: 'error',
          fix_type: 'manual',
        });
      }
    }
  }
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function checkNonTranslatableKeys(
  obj: Record<string, unknown>,
  parentPath: string,
  filePath: string,
  warnings: ValidationError[],
): void {
  for (const key of Object.keys(obj)) {
    if (NON_TRANSLATABLE_KEYS.has(key)) {
      const fieldPath = parentPath ? `${parentPath}.${key}` : key;
      warnings.push({
        file_path: filePath,
        field_path: fieldPath,
        error_code: ErrorCodes.TRANSLATION_NON_TRANSLATABLE_KEY,
        message: `Key '${key}' is not translatable and should not appear in translation files`,
        severity: 'warning',
        fix_type: 'manual',
      });
    }
  }
}

// ---------------------------------------------------------------------------
// Partition & batch helpers
// ---------------------------------------------------------------------------

/**
 * Partition a file map into translation files and non-translation files.
 */
export function partitionTranslationFiles(
  files: Record<string, string>,
): { translationFiles: Record<string, string>; nonTranslationFiles: Record<string, string> } {
  const translationFiles: Record<string, string> = {};
  const nonTranslationFiles: Record<string, string> = {};

  for (const [relativePath, content] of Object.entries(files)) {
    if (isTranslationFile(relativePath)) {
      translationFiles[relativePath] = content;
    } else {
      nonTranslationFiles[relativePath] = content;
    }
  }

  return { translationFiles, nonTranslationFiles };
}

/**
 * Validate all translation files and return a results map.
 *
 * `external` supplies types from the loaded type catalogs; without it, keys
 * inherited from a parent outside the package are not checked.
 */
export function validateTranslationFiles(
  translationFiles: Record<string, string>,
  allFiles: Record<string, string>,
  external?: ExternalSchemas,
): Record<string, ValidationResult> {
  const ctx = buildSchemaContext(allFiles, external);
  const results: Record<string, ValidationResult> = {};

  for (const [filePath, content] of Object.entries(translationFiles)) {
    results[filePath] = validateTranslationFile(filePath, content, allFiles, ctx);
  }

  return results;
}

/**
 * Extract deduplicated, sorted locale codes from translation file paths.
 */
export function extractLocales(translationPaths: string[]): string[] {
  const locales = new Set<string>();

  for (const p of translationPaths) {
    const locale = parseTranslationLocale(path.basename(p));
    if (locale) locales.add(locale);
  }

  return [...locales].sort();
}
