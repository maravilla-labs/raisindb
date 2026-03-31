// Field types matching the Rust FieldSchema enum
export type FieldType =
  | 'TextField'
  | 'RichTextField'
  | 'NumberField'
  | 'DateField'
  | 'LocationField'
  | 'BooleanField'
  | 'MediaField'
  | 'ReferenceField'
  | 'TagField'
  | 'OptionsField'
  | 'JsonObjectField'
  | 'CompositeField'
  | 'SectionField'
  | 'ElementField'
  | 'ListingField'

// Base field schema that all fields share (flattened into each field type via serde flatten)
export interface FieldTypeSchema {
  name: string
  title?: string
  label?: string
  required?: boolean
  description?: string
  help_text?: string
  default_value?: any
  validations?: string[]
  is_hidden?: boolean
  multiple?: boolean
  design_value?: boolean
  translatable?: boolean
}

// Internal ID for React keys (not serialized to API)
interface InternalFieldId {
  id?: string
}

// Field-specific configurations (aligned with raisin-models)

export interface TextFieldConfig {
  /** Maximum length for text */
  max_length?: number
}

export interface RichTextFieldConfig {
  /** Maximum length for rich text */
  max_length?: number
}

export interface NumberFieldConfig {
  /** True for integers, false for decimals */
  is_integer?: boolean
  /** Minimum value */
  min_value?: number
  /** Maximum value */
  max_value?: number
}

export type DateMode = 'DateTime' | 'Date' | 'Time' | 'TimeRange'

export interface DateFieldConfig {
  /** ISO 8601 or custom format */
  date_format?: string
  /** Picker type */
  date_mode?: DateMode
}

export interface MediaFieldConfig {
  /** Allowed media types (e.g., ["image", "video"]) */
  allowed_types?: string[]
}

export interface ReferenceFieldConfig {
  /** Types of referenced entries */
  allowed_entry_types?: string[]
}

export interface TagFieldConfig {
  /** Allowed tags */
  allowed_tags?: string[]
  /** Maximum number of tags */
  max_tags?: number
}

export type OptionsRenderType = 'Dropdown' | 'Radio' | 'Checkbox'

export interface OptionsFieldConfig {
  /** Available options for selection */
  options: string[]
  /** How options are rendered */
  render_as?: OptionsRenderType
  /** Allow multiple selections */
  multi_select?: boolean
}

export interface ListingFieldConfig {
  /** Types of referenced entries */
  allowed_entry_types?: string[]
  /** Field to sort by */
  sort_by?: string
  /** Ascending or descending */
  sort_order?: string
  /** Maximum number of entries to show */
  limit?: number
}

// Layout node types for form layout configuration
export type LayoutNode =
  | {
      type: 'Container'
      direction?: 'horizontal' | 'vertical'
      spacing?: number
      alignment?: string
      children: LayoutNode[]
    }
  | {
      type: 'Group'
      label?: string
      direction?: 'horizontal' | 'vertical'
      spacing?: number
      children: LayoutNode[]
    }
  | {
      type: 'TabPanel'
      tabs: Array<{ label: string; children: LayoutNode[] }>
    }
  | {
      type: 'Field'
      name: string
      width?: string
      condition?: string
    }
  | {
      type: 'Grid'
      rows?: number
      columns?: number
      children: LayoutNode[]
    }

// Field schema types matching Rust FieldSchema enum
// Note: Rust uses #[serde(flatten)] on base, so base fields are at root level
export type FieldSchema =
  | ({ $type: 'TextField'; config?: TextFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'RichTextField'; config?: RichTextFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'NumberField'; config?: NumberFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'DateField'; config?: DateFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'LocationField' } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'BooleanField' } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'MediaField'; config?: MediaFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'ReferenceField'; config?: ReferenceFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'TagField'; config?: TagFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'OptionsField'; config?: OptionsFieldConfig } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'JsonObjectField' } & FieldTypeSchema & InternalFieldId)
  | ({
      $type: 'CompositeField'
      /** Nested fields within this composite field */
      fields?: FieldSchema[]
      /** Layout configuration for the nested fields */
      layout?: LayoutNode[]
    } & FieldTypeSchema &
      InternalFieldId)
  | ({
      $type: 'SectionField'
      allowed_element_types?: string[]
      render_as?: string
    } & FieldTypeSchema &
      InternalFieldId)
  | ({ $type: 'ElementField'; element_type: string } & FieldTypeSchema & InternalFieldId)
  | ({ $type: 'ListingField'; config?: ListingFieldConfig } & FieldTypeSchema & InternalFieldId)

// Archetype definition
export interface ArchetypeDefinition {
  id?: string
  name: string
  extends?: string
  strict?: boolean  // Disallow undefined properties
  icon?: string
  title?: string
  description?: string
  base_node_type?: string
  fields: FieldSchema[]
  initial_content?: Record<string, unknown>
  layout?: LayoutNode[]
  meta?: Record<string, unknown>
  version?: number
  created_at?: string
  updated_at?: string
  published_at?: string
  published_by?: string
  publishable?: boolean
  previous_version?: string
}

// Element type definition (for ElementField/SectionField/CompositeField selection)
export interface ElementType {
  id?: string
  name: string
  title?: string
  icon?: string
  description?: string
  fields: FieldSchema[]
  layout?: LayoutNode[]
  meta?: Record<string, unknown>
  version?: number
  publishable?: boolean
}
