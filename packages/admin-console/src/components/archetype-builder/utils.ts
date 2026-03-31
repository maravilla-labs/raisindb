import { nanoid } from 'nanoid'
import * as yaml from 'js-yaml'
import type { ArchetypeDefinition, FieldSchema, FieldType } from './types'

/**
 * Generate a unique ID for internal use
 */
function generateId(): string {
  return nanoid()
}

/**
 * Add internal IDs to field schemas for React keys and drag-drop
 */
export function addFieldIds(field: any): FieldSchema {
  const fieldWithId = { ...field, id: generateId() }

  // Handle CompositeField with inline fields
  if (field.$type === 'CompositeField' && Array.isArray(field.fields)) {
    fieldWithId.fields = field.fields.map((f: any) => addFieldIds(f))
  }

  return fieldWithId as FieldSchema
}

/**
 * Remove internal IDs from field schemas before serialization
 */
export function cleanFieldIds(field: FieldSchema): any {
  const { id, ...rest } = field as any

  // Handle CompositeField with inline fields
  if (rest.$type === 'CompositeField' && Array.isArray(rest.fields)) {
    rest.fields = rest.fields.map((f: FieldSchema) => cleanFieldIds(f))
  }

  return rest
}

/**
 * Parse YAML content to ArchetypeDefinition, adding internal IDs
 */
export function parseYamlToArchetype(yamlContent: string): ArchetypeDefinition {
  const parsed = yaml.load(yamlContent) as any

  if (!parsed || typeof parsed !== 'object') {
    throw new Error('Invalid YAML: must be an object')
  }

  const archetype: ArchetypeDefinition = {
    ...parsed,
    fields: Array.isArray(parsed.fields) ? parsed.fields.map(addFieldIds) : [],
  }

  return archetype
}

/**
 * Serialize ArchetypeDefinition to YAML, removing internal IDs
 */
export function serializeArchetypeToYaml(archetype: ArchetypeDefinition): string {
  const cleaned = {
    ...archetype,
    fields: archetype.fields.map(cleanFieldIds),
  }

  // Remove internal properties
  delete (cleaned as any).id
  delete (cleaned as any).created_at
  delete (cleaned as any).updated_at
  delete (cleaned as any).published_at
  delete (cleaned as any).published_by
  delete (cleaned as any).previous_version
  delete (cleaned as any).version

  // Remove undefined/empty values
  Object.keys(cleaned).forEach((key) => {
    const value = (cleaned as any)[key]
    if (value === undefined || value === null || value === '') {
      delete (cleaned as any)[key]
    }
  })

  return yaml.dump(cleaned, { indent: 2, lineWidth: -1 })
}

/**
 * Create a new field with default values based on type
 * Note: Fields use flat structure matching Rust's serde(flatten) on base
 */
export function createNewField(type: FieldType): FieldSchema {
  const id = generateId()

  switch (type) {
    case 'TextField':
      return {
        $type: 'TextField',
        id,
        name: '',
        required: false,
        translatable: true,
        config: {},
      }

    case 'RichTextField':
      return {
        $type: 'RichTextField',
        id,
        name: '',
        required: false,
        translatable: true,
        config: {},
      }

    case 'NumberField':
      return {
        $type: 'NumberField',
        id,
        name: '',
        required: false,
        config: {},
      }

    case 'DateField':
      return {
        $type: 'DateField',
        id,
        name: '',
        required: false,
        config: {},
      }

    case 'LocationField':
      return {
        $type: 'LocationField',
        id,
        name: '',
        required: false,
      }

    case 'BooleanField':
      return {
        $type: 'BooleanField',
        id,
        name: '',
        required: false,
      }

    case 'MediaField':
      return {
        $type: 'MediaField',
        id,
        name: '',
        required: false,
        multiple: false,
        config: {},
      }

    case 'ReferenceField':
      return {
        $type: 'ReferenceField',
        id,
        name: '',
        required: false,
        config: {},
      }

    case 'TagField':
      return {
        $type: 'TagField',
        id,
        name: '',
        required: false,
        multiple: true,
        config: {},
      }

    case 'OptionsField':
      return {
        $type: 'OptionsField',
        id,
        name: '',
        required: false,
        config: {
          options: [],
        },
      }

    case 'JsonObjectField':
      return {
        $type: 'JsonObjectField',
        id,
        name: '',
        required: false,
      }

    case 'CompositeField':
      return {
        $type: 'CompositeField',
        id,
        name: '',
        required: false,
        translatable: true,
        fields: [],
      }

    case 'SectionField':
      return {
        $type: 'SectionField',
        id,
        name: '',
        required: false,
        translatable: true,
        allowed_element_types: [],
      }

    case 'ElementField':
      return {
        $type: 'ElementField',
        id,
        name: '',
        required: false,
        translatable: true,
        element_type: '',
      }

    case 'ListingField':
      return {
        $type: 'ListingField',
        id,
        name: '',
        required: false,
        config: {},
      }

    default:
      throw new Error(`Unknown field type: ${type}`)
  }
}

/**
 * Validate archetype definition
 */
export function validateArchetype(archetype: ArchetypeDefinition): Record<string, string> {
  const errors: Record<string, string> = {}

  // Validate name
  if (!archetype.name || archetype.name.trim().length === 0) {
    errors.name = 'Name is required'
  } else if (!/^[a-z][a-z0-9]*:[A-Z][a-zA-Z0-9]*$/.test(archetype.name)) {
    errors.name = 'Name must follow pattern: namespace:ArchetypeName (e.g., marketing:HeroSection)'
  }

  // Validate extends
  if (archetype.extends && !/^[a-z][a-z0-9]*:[A-Z][a-zA-Z0-9]*$/.test(archetype.extends)) {
    errors.extends = 'Extends must follow pattern: namespace:ArchetypeName'
  }

  // Validate fields - now using flat structure (field.name instead of field.base.name)
  const fieldNames = new Set<string>()
  archetype.fields.forEach((field, index) => {
    const fieldName = field.name

    if (!fieldName || fieldName.trim().length === 0) {
      errors[`field_${index}_name`] = 'Field name is required'
    } else if (!/^[a-z][a-z0-9_]*$/.test(fieldName)) {
      errors[`field_${index}_name`] = 'Field name must be lowercase with underscores (e.g., hero_title)'
    } else if (fieldNames.has(fieldName)) {
      errors[`field_${index}_name`] = `Duplicate field name: ${fieldName}`
    } else {
      fieldNames.add(fieldName)
    }

    // Validate ElementField
    if (field.$type === 'ElementField' && !field.element_type) {
      errors[`field_${index}_element_type`] = 'Element type is required'
    }
  })

  return errors
}

/**
 * Clean undefined and empty values from an object
 */
export function cleanObject<T>(value: T): T {
  if (Array.isArray(value)) {
    return value
      .map((item) => cleanObject(item))
      .filter((item) => item !== undefined && item !== null) as unknown as T
  }

  if (value && typeof value === 'object') {
    const result: Record<string, unknown> = {}
    Object.entries(value as Record<string, unknown>).forEach(([key, val]) => {
      if (val === undefined || val === null || val === '') {
        return
      }
      result[key] = cleanObject(val)
    })
    return result as unknown as T
  }

  return value
}
