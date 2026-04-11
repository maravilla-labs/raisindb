/**
 * TypeScript types for the schema validation WASM module
 */

/** Type of fix that can be applied */
export type FixType = 'auto_fixable' | 'needs_input' | 'manual';

/** Severity level of a validation issue */
export type Severity = 'error' | 'warning';

/** Type of file being validated */
export type FileType = 'manifest' | 'nodetype' | 'workspace' | 'content' | 'archetype' | 'elementtype' | 'translation';

/** Suggested fix for a validation error */
export interface SuggestedFix {
  /** Human-readable description of the fix */
  description: string;
  /** The original value (if applicable) */
  old_value?: string;
  /** The suggested new value (for AutoFixable) */
  new_value?: string;
  /** Available options to choose from (for NeedsInput) */
  options?: string[];
}

/** A single validation error or warning */
export interface ValidationError {
  /** Path to the file being validated */
  file_path: string;
  /** Line number (1-based, if available) */
  line?: number;
  /** Column number (1-based, if available) */
  column?: number;
  /** JSON path to the field with the error (e.g., "properties[0].name") */
  field_path: string;
  /** Machine-readable error code (e.g., "INVALID_NODE_TYPE_NAME") */
  error_code: string;
  /** Human-readable error message */
  message: string;
  /** Severity level */
  severity: Severity;
  /** Type of fix available */
  fix_type: FixType;
  /** Suggested fix (if available) */
  suggested_fix?: SuggestedFix;
}

/** Result of validating a single file */
export interface ValidationResult {
  /** Whether validation passed (no errors, warnings ok) */
  success: boolean;
  /** Type of file validated */
  file_type: FileType;
  /** List of errors (block creation) */
  errors: ValidationError[];
  /** List of warnings (allow creation) */
  warnings: ValidationError[];
}

/** Result of validating an entire package */
export type PackageValidationResults = Record<string, ValidationResult>;

/** Error codes used in validation */
export const ErrorCodes = {
  // YAML errors
  YAML_SYNTAX_ERROR: 'YAML_SYNTAX_ERROR',
  YAML_PARSE_ERROR: 'YAML_PARSE_ERROR',

  // Manifest errors
  MISSING_REQUIRED_FIELD: 'MISSING_REQUIRED_FIELD',
  INVALID_PACKAGE_NAME: 'INVALID_PACKAGE_NAME',
  INVALID_VERSION: 'INVALID_VERSION',

  // NodeType errors
  INVALID_NODE_TYPE_NAME: 'INVALID_NODE_TYPE_NAME',
  INVALID_PROPERTY_TYPE: 'INVALID_PROPERTY_TYPE',
  DUPLICATE_PROPERTY: 'DUPLICATE_PROPERTY',
  INVALID_EXTENDS: 'INVALID_EXTENDS',
  INVALID_MIXIN: 'INVALID_MIXIN',

  // Workspace errors
  INVALID_WORKSPACE_NAME: 'INVALID_WORKSPACE_NAME',
  INVALID_ALLOWED_TYPE: 'INVALID_ALLOWED_TYPE',

  // Content errors
  INVALID_CONTENT_NODE_TYPE: 'INVALID_CONTENT_NODE_TYPE',
  MISSING_REQUIRED_PROPERTY: 'MISSING_REQUIRED_PROPERTY',
  INVALID_PROPERTY_VALUE: 'INVALID_PROPERTY_VALUE',

  // Translation errors/warnings
  TRANSLATION_INVALID_YAML: 'TRANSLATION_INVALID_YAML',
  TRANSLATION_NOT_OBJECT: 'TRANSLATION_NOT_OBJECT',
  TRANSLATION_MISSING_BASE_NODE: 'TRANSLATION_MISSING_BASE_NODE',
  TRANSLATION_NON_TRANSLATABLE_KEY: 'TRANSLATION_NON_TRANSLATABLE_KEY',
  TRANSLATION_FIELD_NOT_TRANSLATABLE: 'TRANSLATION_FIELD_NOT_TRANSLATABLE',

  // Composite field UUID validation
  COMPOSITE_MISSING_UUID: 'COMPOSITE_MISSING_UUID',
  COMPOSITE_DUPLICATE_UUID: 'COMPOSITE_DUPLICATE_UUID',

  // Reference warnings
  UNKNOWN_NODE_TYPE_REFERENCE: 'UNKNOWN_NODE_TYPE_REFERENCE',
  UNKNOWN_WORKSPACE_REFERENCE: 'UNKNOWN_WORKSPACE_REFERENCE',
  UNRESOLVABLE_CONTENT_REFERENCE: 'UNRESOLVABLE_CONTENT_REFERENCE',
  UNKNOWN_ARCHETYPE_REFERENCE: 'UNKNOWN_ARCHETYPE_REFERENCE',
  UNKNOWN_ELEMENT_TYPE_REFERENCE: 'UNKNOWN_ELEMENT_TYPE_REFERENCE',
} as const;
