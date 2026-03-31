/**
 * Validation error with position information for Monaco editor markers
 */
export interface ValidationError {
  /** Line number (1-based for Monaco) */
  line: number
  /** Column number (1-based for Monaco) */
  column: number
  /** End line number */
  end_line: number
  /** End column number */
  end_column: number
  /** Error message */
  message: string
  /** Severity: "error" or "warning" */
  severity: 'error' | 'warning'
}

/**
 * Result of SQL validation from WASM module
 */
export interface ValidationResult {
  /** Whether the SQL is valid */
  success: boolean
  /** List of validation errors */
  errors: ValidationError[]
}

// =============================================================================
// Table Catalog Types
// =============================================================================

/**
 * Column definition for table catalog
 */
export interface ColumnDef {
  /** Column name */
  name: string
  /** Data type (e.g., "String", "Number", "Boolean") */
  data_type: string
  /** Whether the column is nullable */
  nullable: boolean
}

/**
 * Table definition for catalog
 */
export interface TableDef {
  /** Table name */
  name: string
  /** Column definitions */
  columns: ColumnDef[]
}

// =============================================================================
// Completion Types
// =============================================================================

/**
 * Completion item from WASM
 */
export interface CompletionItem {
  label: string
  kind: string
  detail?: string
  documentation?: string
  insert_text: string
  insert_text_format: string
  sort_text?: string
  filter_text?: string
}

/**
 * Completion result from WASM
 */
export interface CompletionResult {
  items: CompletionItem[]
  is_incomplete: boolean
}

/**
 * Function signature from WASM
 */
export interface FunctionSignatureInfo {
  name: string
  params: string[]
  return_type: string
  category: string
  is_deterministic: boolean
}

// =============================================================================
// Worker Messages
// =============================================================================

/**
 * Messages sent to the validation worker
 */
export type ValidatorWorkerRequest =
  | { type: 'validate'; id: number; sql: string }
  | { type: 'set_catalog'; catalog: TableDef[] }
  | { type: 'clear_catalog' }
  | { type: 'completion'; id: number; sql: string; offset: number }
  | { type: 'signatures'; id: number; functionName: string }
  | { type: 'all_functions'; id: number }

/**
 * Messages received from the validation worker
 */
export type ValidatorWorkerResponse =
  | { type: 'ready' }
  | { type: 'result'; id: number; result?: ValidationResult; error?: string }
  | { type: 'catalog_set'; success: boolean; error?: string }
  | { type: 'catalog_cleared' }
  | { type: 'completion'; id: number; result?: CompletionResult; error?: string }
  | { type: 'signatures'; id: number; result?: FunctionSignatureInfo[]; error?: string }
  | { type: 'all_functions'; id: number; result?: FunctionSignatureInfo[]; error?: string }
