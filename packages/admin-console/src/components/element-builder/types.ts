/**
 * Element Type Builder Types
 *
 * Reuses FieldSchema from archetype-builder since ElementType
 * uses the same field types as Archetype.
 */

// Re-export shared types from archetype-builder
export type { FieldSchema, FieldType, LayoutNode } from '../archetype-builder/types'

/**
 * Element type definition for the visual builder.
 * Extends the base ElementType with internal IDs for fields.
 */
export interface ElementTypeDefinition {
  id?: string
  name: string
  title?: string
  icon?: string
  description?: string
  extends?: string  // Parent element type for inheritance
  strict?: boolean  // Disallow undefined properties
  fields: import('../archetype-builder/types').FieldSchema[]
  layout?: import('../archetype-builder/types').LayoutNode[]
  meta?: Record<string, unknown>
  initial_content?: Record<string, unknown>
  version?: number
  created_at?: string
  updated_at?: string
  published_at?: string
  published_by?: string
  publishable?: boolean
  previous_version?: string
}
