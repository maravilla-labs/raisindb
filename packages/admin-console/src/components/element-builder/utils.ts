/**
 * Element Type Builder Utilities
 *
 * Reuses field utilities from archetype-builder since ElementType
 * uses the same FieldSchema.
 */

import * as yaml from 'js-yaml'
import type { ElementTypeDefinition } from './types'

// Re-export shared utilities from archetype-builder
export {
  addFieldIds,
  cleanFieldIds,
  createNewField,
  cleanObject,
} from '../archetype-builder/utils'

import { addFieldIds, cleanFieldIds } from '../archetype-builder/utils'

/**
 * Parse YAML content to ElementTypeDefinition, adding internal IDs
 */
export function parseYamlToElementType(yamlContent: string): ElementTypeDefinition {
  const parsed = yaml.load(yamlContent) as any

  if (!parsed || typeof parsed !== 'object') {
    throw new Error('Invalid YAML: must be an object')
  }

  const elementType: ElementTypeDefinition = {
    ...parsed,
    fields: Array.isArray(parsed.fields) ? parsed.fields.map(addFieldIds) : [],
  }

  return elementType
}

/**
 * Serialize ElementTypeDefinition to YAML, removing internal IDs
 */
export function serializeElementTypeToYaml(elementType: ElementTypeDefinition): string {
  const cleaned = {
    ...elementType,
    fields: elementType.fields.map(cleanFieldIds),
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
 * Validate element type definition
 */
export function validateElementType(elementType: ElementTypeDefinition): Record<string, string> {
  const errors: Record<string, string> = {}

  // Validate name
  if (!elementType.name || elementType.name.trim().length === 0) {
    errors.name = 'Name is required'
  } else if (!/^[a-z][a-z0-9]*:[A-Z][a-zA-Z0-9]*$/.test(elementType.name)) {
    errors.name = 'Name must follow pattern: namespace:ElementName (e.g., marketing:HeroBlock)'
  }

  // Validate fields
  const fieldNames = new Set<string>()
  elementType.fields.forEach((field, index) => {
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
