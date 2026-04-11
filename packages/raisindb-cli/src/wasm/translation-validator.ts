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
  fields: FieldDef[];
}

interface ElementTypeSchema {
  name: string;
  fields: FieldDef[];
}

interface NodeTypeProp {
  name: string;
  is_translatable?: boolean;
}

interface NodeTypeSchema {
  name: string;
  properties: NodeTypeProp[];
}

/** Schema context built from all package files. */
export interface SchemaContext {
  archetypes: Map<string, ArchetypeSchema>;
  elementTypes: Map<string, ElementTypeSchema>;
  nodeTypes: Map<string, NodeTypeSchema>;
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
export function buildSchemaContext(allFiles: Record<string, string>): SchemaContext {
  const archetypes = new Map<string, ArchetypeSchema>();
  const elementTypes = new Map<string, ElementTypeSchema>();
  const nodeTypes = new Map<string, NodeTypeSchema>();

  for (const [filePath, content] of Object.entries(allFiles)) {
    try {
      const parsed = yaml.parse(content);
      if (!parsed || typeof parsed !== 'object') continue;

      if (filePath.startsWith('archetypes/') && parsed.name && parsed.fields) {
        archetypes.set(parsed.name, parsed as ArchetypeSchema);
      } else if (filePath.startsWith('elementtypes/') && parsed.name && parsed.fields) {
        elementTypes.set(parsed.name, parsed as ElementTypeSchema);
      } else if (filePath.startsWith('nodetypes/') && parsed.name && parsed.properties) {
        nodeTypes.set(parsed.name, parsed as NodeTypeSchema);
      }
    } catch {
      // Skip unparseable files — they'll be caught by the WASM validator
    }
  }

  return { archetypes, elementTypes, nodeTypes };
}

/**
 * Collect the set of translatable field names from a list of FieldDefs.
 * Recurses into CompositeField sub-fields.
 */
function collectTranslatableFields(fields: FieldDef[]): Set<string> {
  const result = new Set<string>();
  for (const f of fields) {
    if (f.translatable === true) {
      result.add(f.name);
    }
    // CompositeField sub-fields: their translatable children apply to items
    // within the composite array (handled separately during validation).
  }
  return result;
}

/**
 * Find an archetype by the node's `archetype` value, falling back to
 * finding one whose `base_node_type` matches the node's `node_type`.
 */
function resolveArchetype(
  nodeType: string | undefined,
  archetypeName: string | undefined,
  ctx: SchemaContext,
): ArchetypeSchema | undefined {
  if (archetypeName) {
    const a = ctx.archetypes.get(archetypeName);
    if (a) return a;
  }
  if (nodeType) {
    for (const a of ctx.archetypes.values()) {
      if (a.base_node_type === nodeType) return a;
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
  // Build uuid → element_type map from base items
  const uuidToElementType = new Map<string, string>();
  for (const baseItem of baseItems) {
    if (baseItem && typeof baseItem === 'object' && !Array.isArray(baseItem)) {
      const rec = baseItem as Record<string, unknown>;
      if (typeof rec.uuid === 'string' && typeof rec.element_type === 'string') {
        uuidToElementType.set(rec.uuid, rec.element_type);
      }
    }
  }

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (!item || typeof item !== 'object' || Array.isArray(item)) continue;
    const rec = item as Record<string, unknown>;

    // Resolve element_type from base items via uuid
    const uuid = rec.uuid as string | undefined;
    const elementTypeName = uuid ? uuidToElementType.get(uuid) : undefined;
    if (!elementTypeName) continue;

    const etSchema = ctx.elementTypes.get(elementTypeName);
    if (!etSchema) continue; // unknown element type — warned elsewhere

    const etTranslatable = collectTranslatableFields(etSchema.fields);

    for (const key of Object.keys(rec)) {
      if (NON_TRANSLATABLE_KEYS.has(key)) continue;
      if (SECTION_IDENTIFIER_KEYS.has(key)) continue;

      // Check CompositeField sub-arrays inside elements
      const etFieldDef = etSchema.fields.find(f => f.name === key);
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
  const subTranslatable = collectTranslatableFields(compositeDef.fields);

  // If composite has translatable sub-fields, items need unique UUIDs
  if (subTranslatable.size > 0) {
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
 */
export function validateTranslationFiles(
  translationFiles: Record<string, string>,
  allFiles: Record<string, string>,
): Record<string, ValidationResult> {
  const ctx = buildSchemaContext(allFiles);
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
