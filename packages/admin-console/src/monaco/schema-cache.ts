/**
 * Schema Cache for SQL Completions
 *
 * Stores table/column information locally for fast autocomplete lookups.
 * This cache is populated from the workspaces API and WASM function registry.
 */

// =============================================================================
// Types
// =============================================================================

export interface CachedColumn {
  name: string
  dataType: string
  nullable: boolean
}

export interface CachedTable {
  name: string
  displayName: string
  columns: CachedColumn[]
  isWorkspace: boolean
}

export interface FunctionSignature {
  name: string
  params: string[]
  returnType: string
  category: string
  isDeterministic: boolean
}

// =============================================================================
// Built-in Schema
// =============================================================================

/**
 * Standard columns available on all workspace tables (from nodes schema)
 */
const NODES_COLUMNS: CachedColumn[] = [
  { name: 'id', dataType: 'Text', nullable: false },
  { name: 'path', dataType: 'Path', nullable: false },
  { name: 'name', dataType: 'Text', nullable: false },
  { name: 'node_type', dataType: 'Text', nullable: false },
  { name: 'archetype', dataType: 'Text', nullable: true },
  { name: 'properties', dataType: 'JsonB', nullable: false },
  { name: 'parent_name', dataType: 'Text', nullable: true },
  { name: 'version', dataType: 'Int', nullable: false },
  { name: 'created_at', dataType: 'Timestamp', nullable: false },
  { name: 'updated_at', dataType: 'Timestamp', nullable: false },
  { name: 'published_at', dataType: 'Timestamp', nullable: true },
  { name: 'published_by', dataType: 'Text', nullable: true },
  { name: 'updated_by', dataType: 'Text', nullable: true },
  { name: 'created_by', dataType: 'Text', nullable: true },
  { name: 'translations', dataType: 'JsonB', nullable: true },
  { name: 'owner_id', dataType: 'Text', nullable: true },
  { name: 'relations', dataType: 'JsonB', nullable: true },
  // Generated columns
  { name: 'parent_path', dataType: 'Path', nullable: true },
  { name: 'depth', dataType: 'Int', nullable: false },
  { name: '__revision', dataType: 'BigInt', nullable: true },
  { name: '__branch', dataType: 'Text', nullable: false },
  { name: '__workspace', dataType: 'Text', nullable: false },
  { name: 'locale', dataType: 'Text', nullable: false },
]

/**
 * NodeTypes schema table columns
 */
const NODE_TYPES_COLUMNS: CachedColumn[] = [
  { name: 'type_name', dataType: 'Text', nullable: false },
  { name: 'extends', dataType: 'Text', nullable: true },
  { name: 'properties', dataType: 'JsonB', nullable: false },
  { name: 'is_abstract', dataType: 'Boolean', nullable: false },
  { name: 'created_at', dataType: 'Timestamp', nullable: false },
]

/**
 * Archetypes schema table columns
 */
const ARCHETYPES_COLUMNS: CachedColumn[] = [
  { name: 'archetype_name', dataType: 'Text', nullable: false },
  { name: 'base_node_type', dataType: 'Text', nullable: false },
  { name: 'title', dataType: 'Text', nullable: true },
  { name: 'fields', dataType: 'JsonB', nullable: false },
  { name: 'created_at', dataType: 'Timestamp', nullable: false },
]

/**
 * ElementTypes schema table columns
 */
const ELEMENT_TYPES_COLUMNS: CachedColumn[] = [
  { name: 'type_name', dataType: 'Text', nullable: false },
  { name: 'properties', dataType: 'JsonB', nullable: false },
  { name: 'created_at', dataType: 'Timestamp', nullable: false },
]

// =============================================================================
// Schema Cache Class
// =============================================================================

export class SchemaCache {
  private tables: Map<string, CachedTable> = new Map()
  private functions: Map<string, FunctionSignature[]> = new Map()
  private lastUpdate: number = 0
  private ttl: number = 5 * 60 * 1000 // 5 minutes TTL

  constructor() {
    this.initializeBuiltinSchema()
  }

  /**
   * Initialize with built-in tables (nodes, NodeTypes, etc.)
   */
  private initializeBuiltinSchema() {
    // Add 'nodes' table
    this.tables.set('nodes', {
      name: 'nodes',
      displayName: 'nodes',
      columns: NODES_COLUMNS,
      isWorkspace: false,
    })

    // Add schema tables
    this.tables.set('nodetypes', {
      name: 'NodeTypes',
      displayName: 'NodeTypes',
      columns: NODE_TYPES_COLUMNS,
      isWorkspace: false,
    })

    this.tables.set('archetypes', {
      name: 'Archetypes',
      displayName: 'Archetypes',
      columns: ARCHETYPES_COLUMNS,
      isWorkspace: false,
    })

    this.tables.set('elementtypes', {
      name: 'ElementTypes',
      displayName: 'ElementTypes',
      columns: ELEMENT_TYPES_COLUMNS,
      isWorkspace: false,
    })
  }

  /**
   * Update cache with workspace tables
   */
  updateWorkspaces(workspaceNames: string[]) {
    for (const name of workspaceNames) {
      const key = name.toLowerCase()
      // Workspace tables have same schema as 'nodes'
      this.tables.set(key, {
        name: name,
        displayName: name,
        columns: [...NODES_COLUMNS],
        isWorkspace: true,
      })
    }
    this.lastUpdate = Date.now()
  }

  /**
   * Set function signatures (replaces existing)
   */
  setFunctions(signatures: FunctionSignature[]) {
    this.functions.clear()
    this.updateFunctions(signatures)
  }

  /**
   * Update cache with function signatures from WASM
   */
  updateFunctions(signatures: FunctionSignature[]) {
    // Group by function name (some functions have multiple overloads)
    for (const sig of signatures) {
      const key = sig.name.toUpperCase()
      const existing = this.functions.get(key) ?? []
      existing.push(sig)
      this.functions.set(key, existing)
    }
  }

  // =========================================================================
  // Table Lookups
  // =========================================================================

  /**
   * Get table by name (case-insensitive)
   */
  getTable(name: string): CachedTable | undefined {
    return this.tables.get(name.toLowerCase())
  }

  /**
   * Get all tables
   */
  getAllTables(): CachedTable[] {
    return Array.from(this.tables.values())
  }

  /**
   * Get all table names for completion
   */
  getTableNames(): string[] {
    return this.getAllTables().map((t) => t.displayName)
  }

  /**
   * Get columns for a table (case-insensitive)
   */
  getColumnsForTable(tableName: string): CachedColumn[] {
    const table = this.getTable(tableName)
    return table?.columns ?? []
  }

  /**
   * Check if a table exists
   */
  hasTable(name: string): boolean {
    return this.tables.has(name.toLowerCase())
  }

  // =========================================================================
  // Function Lookups
  // =========================================================================

  /**
   * Get function signatures by name (case-insensitive)
   */
  getFunction(name: string): FunctionSignature[] | undefined {
    return this.functions.get(name.toUpperCase())
  }

  /**
   * Get all functions
   */
  getAllFunctions(): FunctionSignature[] {
    const all: FunctionSignature[] = []
    for (const sigs of this.functions.values()) {
      all.push(...sigs)
    }
    return all
  }

  /**
   * Get function names for completion
   */
  getFunctionNames(): string[] {
    return Array.from(this.functions.keys())
  }

  // =========================================================================
  // Cache Management
  // =========================================================================

  /**
   * Check if cache is stale
   */
  isStale(): boolean {
    return Date.now() - this.lastUpdate > this.ttl
  }

  /**
   * Clear workspace tables (keep built-in)
   */
  clearWorkspaces() {
    for (const [key, table] of this.tables.entries()) {
      if (table.isWorkspace) {
        this.tables.delete(key)
      }
    }
  }

  /**
   * Clear all cached data
   */
  clear() {
    this.tables.clear()
    this.functions.clear()
    this.initializeBuiltinSchema()
  }
}

// =============================================================================
// Singleton Instance
// =============================================================================

let schemaCache: SchemaCache | null = null

/**
 * Get or create the schema cache singleton
 */
export function getSchemaCache(): SchemaCache {
  if (!schemaCache) {
    schemaCache = new SchemaCache()
  }
  return schemaCache
}

/**
 * Initialize the schema cache (idempotent)
 */
export function initializeSchemaCache(): SchemaCache {
  return getSchemaCache()
}
