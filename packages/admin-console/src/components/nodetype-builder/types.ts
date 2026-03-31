// Type definitions for the visual node type builder

// Property path for nested selection (e.g., "specs.dimensions.width")
export type PropertyPath = string

// Helper to convert path to segments
export function pathSegments(path: PropertyPath): string[] {
  return path.split('.')
}

// Helper to get parent path
export function parentPath(path: PropertyPath): PropertyPath | undefined {
  const segments = pathSegments(path)
  if (segments.length <= 1) return undefined
  return segments.slice(0, -1).join('.')
}

// Helper to get leaf name from path
export function leafName(path: PropertyPath): string {
  const segments = pathSegments(path)
  return segments[segments.length - 1]
}

export type PropertyType =
  | 'String'
  | 'Number'
  | 'Boolean'
  | 'Array'
  | 'Object'
  | 'Date'
  | 'URL'
  | 'Reference'
  | 'NodeType'
  | 'Element'
  | 'Composite'
  | 'Resource'

export type IndexType = 'Fulltext' | 'Vector' | 'Property'

// Compound index column definition
export interface CompoundIndexColumn {
  property: string // Property name or system field (__node_type, __created_at, __updated_at)
  ascending?: boolean // Sort direction for ordering column (only applies when has_order_column is true)
}

// Compound index definition for efficient ORDER BY + filter queries
export interface CompoundIndexDefinition {
  name: string // Unique name for the index
  columns: CompoundIndexColumn[] // Columns in order (leading equality, trailing ordering)
  has_order_column: boolean // If true, last column is used for ORDER BY
}

export interface PropertyValueSchema {
  id?: string // Internal ID for React keys and drag-drop
  name?: string
  type: PropertyType
  required?: boolean
  unique?: boolean
  default?: any
  constraints?: Record<string, any>
  structure?: Record<string, PropertyValueSchema> // For Object type (unordered by key)
  items?: PropertyValueSchema // For Array type (single schema for array items)
  fields?: PropertyValueSchema[] // For Composite type (ordered array of nested fields)
  value?: any
  meta?: any
  translatable?: boolean
  is_translatable?: boolean // legacy alias
  allow_additional_properties?: boolean
  index?: IndexType[] // Which indexes this property should be included in
  label?: string
  description?: string
  placeholder?: string
  multiline?: boolean
  enum?: Array<string | { value: string; label?: string }>
  options?: Array<string | { value: string; label?: string }>
  values?: Array<string | { value: string; label?: string }>
}

export interface NodeTypeDefinition {
  id?: string
  name: string
  extends?: string
  icon?: string
  description?: string
  version?: number
  strict?: boolean
  versionable?: boolean
  publishable?: boolean
  auditable?: boolean
  indexable?: boolean // Whether this node type should be indexed
  index_types?: IndexType[] // Which index types are enabled for this node type
  allowed_children: string[]
  required_nodes?: string[]
  properties: PropertyValueSchema[]
  mixins?: string[]
  overrides?: Record<string, any>
  initial_structure?: any
  compound_indexes?: CompoundIndexDefinition[] // Compound indexes for efficient ORDER BY + filter
}

export type EditorMode = 'visual' | 'source'

export interface VisualBuilderState {
  mode: EditorMode
  nodeType: NodeTypeDefinition
  selectedPropertyId?: string
  validationErrors: Record<string, string>
  isDirty: boolean
}
