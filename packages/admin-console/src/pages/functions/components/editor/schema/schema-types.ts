/**
 * Type definitions for the JSON Schema visual editor
 * Aligned with raisin-models PropertyDef and PropertyTypeDef
 */

/** Valid property types (aligned with raisin-models PropertyTypeDef) */
export type SchemaPropertyType =
  | 'string'
  | 'number'
  | 'boolean'
  | 'date'
  | 'url'
  | 'reference'
  | 'resource'
  | 'composite'
  | 'element'
  | 'nodetype'
  | 'object'
  | 'array'

/** Index types for property indexing */
export type IndexType = 'fulltext' | 'vector' | 'property'

/** Default value type (aligned with raisin-models DefaultValue) */
export type DefaultValue =
  | { String: string }
  | { Number: number }
  | { Boolean: boolean }
  | 'Null'

/** Constraints for property validation */
export interface PropertyConstraints {
  /** Minimum value (for number type) */
  min?: number
  /** Maximum value (for number type) */
  max?: number
  /** Regex pattern (for string type) */
  pattern?: string
  /** Minimum length (for string type) */
  minLength?: number
  /** Maximum length (for string type) */
  maxLength?: number
  /** Whether number should be integer (replaces 'integer' type) */
  isInteger?: boolean
}

/** Schema property definition for the visual builder (aligned with PropertyDef) */
export interface SchemaProperty {
  /** Unique ID for drag-drop */
  id: string
  /** Property name (key in the JSON Schema) */
  name: string
  /** Property type */
  type: SchemaPropertyType
  /** Human-readable description (required for AI agents) */
  description: string
  /** Whether this property is required */
  required: boolean
  /** Whether values must be unique across nodes */
  unique?: boolean
  /** Index types for this property (Fulltext, Vector, Property) */
  index?: IndexType[]
  /** Default value for this property */
  default?: DefaultValue
  /** Whether this property is translatable (i18n) */
  translatable?: boolean
  /** Validation constraints */
  constraints?: PropertyConstraints
  /** Human-readable label */
  label?: string
  /** Display order hint */
  order?: number
  /** For object types: allow properties not in schema */
  allowAdditionalProperties?: boolean
  /** Enum values for string type */
  enum?: string[]
  /** Item schema for array type */
  items?: SchemaProperty
  /** Nested properties for object type */
  properties?: SchemaProperty[]
}

/** Full schema being edited in visual mode */
export interface EditableSchema {
  /** Root type is always object */
  type: 'object'
  /** Schema description */
  description?: string
  /** List of properties */
  properties: SchemaProperty[]
  /** Whether to allow additional properties (default false for strict validation) */
  additionalProperties?: boolean
}

/** Validation error from schema validation */
export interface SchemaValidationError {
  /** Path to the error (e.g., "properties.name" or "") */
  path: string
  /** Error message */
  message: string
}

/**
 * Generate a unique ID for drag-drop
 */
export function generateId(): string {
  return `prop-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
}

/**
 * Convert a JSON Schema object to an EditableSchema for the visual builder
 */
export function schemaToEditable(schema: Record<string, unknown> | undefined): EditableSchema {
  if (!schema || typeof schema !== 'object') {
    return {
      type: 'object',
      description: '',
      properties: [],
      additionalProperties: false,
    }
  }

  const properties: SchemaProperty[] = []
  const requiredFields = Array.isArray(schema.required) ? schema.required : []

  if (schema.properties && typeof schema.properties === 'object') {
    for (const [name, propSchema] of Object.entries(schema.properties as Record<string, unknown>)) {
      const prop = propSchemaToProperty(name, propSchema as Record<string, unknown>, requiredFields)
      properties.push(prop)
    }
  }

  return {
    type: 'object',
    description: typeof schema.description === 'string' ? schema.description : '',
    properties,
    additionalProperties: schema.additionalProperties === true,
  }
}

/**
 * Convert a property schema to a SchemaProperty
 */
function propSchemaToProperty(
  name: string,
  propSchema: Record<string, unknown>,
  requiredFields: string[]
): SchemaProperty {
  let type = (propSchema.type as SchemaPropertyType) || 'string'

  // Migrate 'integer' to 'number' with isInteger constraint
  const wasInteger = type === 'integer' as string
  if (wasInteger) {
    type = 'number'
  }

  const property: SchemaProperty = {
    id: generateId(),
    name,
    type,
    description: typeof propSchema.description === 'string' ? propSchema.description : '',
    required: requiredFields.includes(name),
  }

  // Handle new fields from PropertyDef
  if (typeof propSchema.unique === 'boolean') {
    property.unique = propSchema.unique
  }
  if (Array.isArray(propSchema.index)) {
    property.index = propSchema.index.filter((v): v is IndexType =>
      ['fulltext', 'vector', 'property'].includes(v as string)
    )
  }
  if (propSchema.default !== undefined && propSchema.default !== null) {
    property.default = propSchema.default as DefaultValue
  }
  if (typeof propSchema.translatable === 'boolean') {
    property.translatable = propSchema.translatable
  }
  if (typeof propSchema.label === 'string') {
    property.label = propSchema.label
  }
  if (typeof propSchema.order === 'number') {
    property.order = propSchema.order
  }
  if (typeof propSchema.allowAdditionalProperties === 'boolean') {
    property.allowAdditionalProperties = propSchema.allowAdditionalProperties
  }

  // Handle constraints
  const constraints: PropertyConstraints = {}
  if (wasInteger) {
    constraints.isInteger = true
  }
  if (typeof propSchema.minimum === 'number') {
    constraints.min = propSchema.minimum
  }
  if (typeof propSchema.maximum === 'number') {
    constraints.max = propSchema.maximum
  }
  if (typeof propSchema.minLength === 'number') {
    constraints.minLength = propSchema.minLength
  }
  if (typeof propSchema.maxLength === 'number') {
    constraints.maxLength = propSchema.maxLength
  }
  if (typeof propSchema.pattern === 'string') {
    constraints.pattern = propSchema.pattern
  }
  // Also check for nested constraints object
  if (propSchema.constraints && typeof propSchema.constraints === 'object') {
    const c = propSchema.constraints as Record<string, unknown>
    if (typeof c.min === 'number') constraints.min = c.min
    if (typeof c.max === 'number') constraints.max = c.max
    if (typeof c.minLength === 'number') constraints.minLength = c.minLength
    if (typeof c.maxLength === 'number') constraints.maxLength = c.maxLength
    if (typeof c.pattern === 'string') constraints.pattern = c.pattern
    if (typeof c.isInteger === 'boolean') constraints.isInteger = c.isInteger
  }
  if (Object.keys(constraints).length > 0) {
    property.constraints = constraints
  }

  // Handle enum for strings
  if (type === 'string' && Array.isArray(propSchema.enum)) {
    property.enum = propSchema.enum.filter((v): v is string => typeof v === 'string')
  }

  // Handle nested objects
  if (type === 'object' && propSchema.properties && typeof propSchema.properties === 'object') {
    const nestedRequired = Array.isArray(propSchema.required) ? propSchema.required : []
    property.properties = []
    for (const [nestedName, nestedSchema] of Object.entries(propSchema.properties as Record<string, unknown>)) {
      property.properties.push(
        propSchemaToProperty(nestedName, nestedSchema as Record<string, unknown>, nestedRequired)
      )
    }
  }

  // Handle array items
  if (type === 'array' && propSchema.items && typeof propSchema.items === 'object') {
    const itemsSchema = propSchema.items as Record<string, unknown>
    property.items = propSchemaToProperty('items', itemsSchema, [])
  }

  return property
}

/**
 * Convert an EditableSchema back to a JSON Schema object
 */
export function editableToSchema(editable: EditableSchema): Record<string, unknown> {
  const schema: Record<string, unknown> = {
    type: 'object',
  }

  if (editable.description) {
    schema.description = editable.description
  }

  const properties: Record<string, unknown> = {}
  const required: string[] = []

  for (const prop of editable.properties) {
    properties[prop.name] = propertyToSchema(prop)
    if (prop.required) {
      required.push(prop.name)
    }
  }

  if (Object.keys(properties).length > 0) {
    schema.properties = properties
  }

  if (required.length > 0) {
    schema.required = required
  }

  if (editable.additionalProperties === false) {
    schema.additionalProperties = false
  }

  return schema
}

/**
 * Convert a SchemaProperty to a JSON Schema property definition
 */
function propertyToSchema(prop: SchemaProperty): Record<string, unknown> {
  const schema: Record<string, unknown> = {
    type: prop.type,
  }

  if (prop.description) {
    schema.description = prop.description
  }

  // Serialize new PropertyDef fields
  if (prop.unique !== undefined) {
    schema.unique = prop.unique
  }
  if (prop.index && prop.index.length > 0) {
    schema.index = prop.index
  }
  if (prop.default !== undefined) {
    schema.default = prop.default
  }
  if (prop.translatable !== undefined) {
    schema.translatable = prop.translatable
  }
  if (prop.label) {
    schema.label = prop.label
  }
  if (prop.order !== undefined) {
    schema.order = prop.order
  }
  if (prop.allowAdditionalProperties !== undefined) {
    schema.allowAdditionalProperties = prop.allowAdditionalProperties
  }

  // Serialize constraints
  if (prop.constraints) {
    const c = prop.constraints
    // For JSON Schema compatibility, use standard constraint names at top level
    if (c.min !== undefined) schema.minimum = c.min
    if (c.max !== undefined) schema.maximum = c.max
    if (c.minLength !== undefined) schema.minLength = c.minLength
    if (c.maxLength !== undefined) schema.maxLength = c.maxLength
    if (c.pattern !== undefined) schema.pattern = c.pattern
    // Also include as nested constraints object for raisin-models compatibility
    schema.constraints = {
      ...(c.min !== undefined && { min: c.min }),
      ...(c.max !== undefined && { max: c.max }),
      ...(c.minLength !== undefined && { minLength: c.minLength }),
      ...(c.maxLength !== undefined && { maxLength: c.maxLength }),
      ...(c.pattern !== undefined && { pattern: c.pattern }),
      ...(c.isInteger !== undefined && { isInteger: c.isInteger }),
    }
  }

  // Handle enum for strings
  if (prop.type === 'string' && prop.enum && prop.enum.length > 0) {
    schema.enum = prop.enum
  }

  // Handle nested objects
  if (prop.type === 'object' && prop.properties && prop.properties.length > 0) {
    const nestedProperties: Record<string, unknown> = {}
    const nestedRequired: string[] = []

    for (const nestedProp of prop.properties) {
      nestedProperties[nestedProp.name] = propertyToSchema(nestedProp)
      if (nestedProp.required) {
        nestedRequired.push(nestedProp.name)
      }
    }

    schema.properties = nestedProperties
    if (nestedRequired.length > 0) {
      schema.required = nestedRequired
    }
  }

  // Handle array items
  if (prop.type === 'array' && prop.items) {
    schema.items = propertyToSchema(prop.items)
  }

  return schema
}

/**
 * Validate a JSON Schema object
 */
export function validateSchema(schema: Record<string, unknown>): SchemaValidationError[] {
  const errors: SchemaValidationError[] = []

  // Root must be object type
  if (schema.type !== 'object') {
    errors.push({ path: '', message: 'Root schema must be of type "object"' })
  }

  // Validate properties
  if (schema.properties && typeof schema.properties === 'object') {
    for (const [name, propSchema] of Object.entries(schema.properties as Record<string, unknown>)) {
      validateProperty(name, propSchema as Record<string, unknown>, `properties.${name}`, errors)
    }
  }

  // Validate required array
  if (schema.required !== undefined) {
    if (!Array.isArray(schema.required)) {
      errors.push({ path: 'required', message: '"required" must be an array' })
    } else {
      const propertyNames = schema.properties
        ? Object.keys(schema.properties as Record<string, unknown>)
        : []
      for (const reqName of schema.required) {
        if (typeof reqName !== 'string') {
          errors.push({ path: 'required', message: 'Required field names must be strings' })
        } else if (!propertyNames.includes(reqName)) {
          errors.push({ path: 'required', message: `Required field "${reqName}" is not defined in properties` })
        }
      }
    }
  }

  return errors
}

/** All valid property types */
const VALID_TYPES: SchemaPropertyType[] = [
  'string', 'number', 'boolean', 'date', 'url',
  'reference', 'resource', 'composite', 'element',
  'nodetype', 'object', 'array'
]

/** Valid index types */
const VALID_INDEX_TYPES: IndexType[] = ['fulltext', 'vector', 'property']

/**
 * Validate a property schema
 */
function validateProperty(
  name: string,
  propSchema: Record<string, unknown>,
  path: string,
  errors: SchemaValidationError[]
): void {
  // Validate property name
  if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(name)) {
    errors.push({
      path,
      message: `Property name "${name}" must be a valid identifier (start with letter or underscore, contain only alphanumeric and underscore)`,
    })
  }

  // Validate type (also accept 'integer' for backwards compatibility)
  const type = propSchema.type as string
  const validTypesWithLegacy = [...VALID_TYPES, 'integer']
  if (!validTypesWithLegacy.includes(type)) {
    errors.push({
      path: `${path}.type`,
      message: `Invalid type "${type}". Must be one of: ${VALID_TYPES.join(', ')}`,
    })
  }

  // Validate description is present (required for AI agents)
  if (!propSchema.description || typeof propSchema.description !== 'string' || propSchema.description.trim() === '') {
    errors.push({
      path: `${path}.description`,
      message: 'Description is required for AI agents to understand this property',
    })
  }

  // Validate index array
  if (propSchema.index !== undefined) {
    if (!Array.isArray(propSchema.index)) {
      errors.push({ path: `${path}.index`, message: 'Index must be an array' })
    } else {
      for (const idx of propSchema.index) {
        if (!VALID_INDEX_TYPES.includes(idx as IndexType)) {
          errors.push({ path: `${path}.index`, message: `Invalid index type "${idx}". Must be one of: ${VALID_INDEX_TYPES.join(', ')}` })
        }
      }
    }
  }

  // Validate order is a number
  if (propSchema.order !== undefined && typeof propSchema.order !== 'number') {
    errors.push({ path: `${path}.order`, message: 'Order must be a number' })
  }

  // Validate enum for strings
  if (propSchema.type === 'string' && propSchema.enum !== undefined) {
    if (!Array.isArray(propSchema.enum)) {
      errors.push({ path: `${path}.enum`, message: 'Enum must be an array' })
    } else {
      const enumValues = propSchema.enum as unknown[]
      const seen = new Set<string>()
      for (let i = 0; i < enumValues.length; i++) {
        const val = enumValues[i]
        if (typeof val !== 'string') {
          errors.push({ path: `${path}.enum[${i}]`, message: 'Enum values must be strings' })
        } else if (val.trim() === '') {
          errors.push({ path: `${path}.enum[${i}]`, message: 'Enum values cannot be empty' })
        } else if (seen.has(val)) {
          errors.push({ path: `${path}.enum[${i}]`, message: `Duplicate enum value: "${val}"` })
        } else {
          seen.add(val)
        }
      }
    }
  }

  // Validate nested object properties
  if (propSchema.type === 'object' && propSchema.properties) {
    if (typeof propSchema.properties !== 'object') {
      errors.push({ path: `${path}.properties`, message: 'Properties must be an object' })
    } else {
      for (const [nestedName, nestedSchema] of Object.entries(propSchema.properties as Record<string, unknown>)) {
        validateProperty(nestedName, nestedSchema as Record<string, unknown>, `${path}.properties.${nestedName}`, errors)
      }
    }
  }

  // Validate array items
  if (propSchema.type === 'array') {
    if (!propSchema.items) {
      errors.push({ path: `${path}.items`, message: 'Array type must have "items" defined' })
    } else if (typeof propSchema.items !== 'object') {
      errors.push({ path: `${path}.items`, message: 'Items must be a schema object' })
    } else {
      validateProperty('items', propSchema.items as Record<string, unknown>, `${path}.items`, errors)
    }
  }
}

/**
 * Validate an EditableSchema (for visual builder validation)
 */
export function validateEditableSchema(editable: EditableSchema): SchemaValidationError[] {
  const errors: SchemaValidationError[] = []

  // Check for duplicate property names
  const names = new Set<string>()
  for (const prop of editable.properties) {
    if (names.has(prop.name)) {
      errors.push({ path: `properties.${prop.name}`, message: `Duplicate property name: "${prop.name}"` })
    }
    names.add(prop.name)

    // Validate each property
    validateEditableProperty(prop, `properties.${prop.name}`, errors)
  }

  return errors
}

/**
 * Validate a single editable property
 */
function validateEditableProperty(
  prop: SchemaProperty,
  path: string,
  errors: SchemaValidationError[]
): void {
  // Validate property name
  if (!prop.name || prop.name.trim() === '') {
    errors.push({ path, message: 'Property name is required' })
  } else if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(prop.name)) {
    errors.push({
      path,
      message: `Property name "${prop.name}" must be a valid identifier`,
    })
  }

  // Validate type is set and valid
  if (!prop.type) {
    errors.push({ path: `${path}.type`, message: 'Property type is required' })
  } else if (!VALID_TYPES.includes(prop.type)) {
    errors.push({
      path: `${path}.type`,
      message: `Invalid type "${prop.type}". Must be one of: ${VALID_TYPES.join(', ')}`,
    })
  }

  // Validate description is set (required for AI agents)
  if (!prop.description || prop.description.trim() === '') {
    errors.push({ path: `${path}.description`, message: 'Description is required for AI agents to understand this property' })
  }

  // Validate index array
  if (prop.index) {
    for (const idx of prop.index) {
      if (!VALID_INDEX_TYPES.includes(idx)) {
        errors.push({ path: `${path}.index`, message: `Invalid index type "${idx}". Must be one of: ${VALID_INDEX_TYPES.join(', ')}` })
      }
    }
  }

  // Validate order is a number
  if (prop.order !== undefined && typeof prop.order !== 'number') {
    errors.push({ path: `${path}.order`, message: 'Order must be a number' })
  }

  // Validate constraints based on type
  if (prop.constraints) {
    const c = prop.constraints
    if (prop.type === 'number') {
      if (c.min !== undefined && typeof c.min !== 'number') {
        errors.push({ path: `${path}.constraints.min`, message: 'min must be a number' })
      }
      if (c.max !== undefined && typeof c.max !== 'number') {
        errors.push({ path: `${path}.constraints.max`, message: 'max must be a number' })
      }
      if (c.min !== undefined && c.max !== undefined && c.min > c.max) {
        errors.push({ path: `${path}.constraints`, message: 'min cannot be greater than max' })
      }
    }
    if (prop.type === 'string') {
      if (c.minLength !== undefined && (typeof c.minLength !== 'number' || c.minLength < 0)) {
        errors.push({ path: `${path}.constraints.minLength`, message: 'minLength must be a non-negative number' })
      }
      if (c.maxLength !== undefined && (typeof c.maxLength !== 'number' || c.maxLength < 0)) {
        errors.push({ path: `${path}.constraints.maxLength`, message: 'maxLength must be a non-negative number' })
      }
      if (c.minLength !== undefined && c.maxLength !== undefined && c.minLength > c.maxLength) {
        errors.push({ path: `${path}.constraints`, message: 'minLength cannot be greater than maxLength' })
      }
      if (c.pattern !== undefined && typeof c.pattern !== 'string') {
        errors.push({ path: `${path}.constraints.pattern`, message: 'pattern must be a string' })
      }
    }
  }

  // Validate enum values
  if (prop.type === 'string' && prop.enum) {
    const seen = new Set<string>()
    for (let i = 0; i < prop.enum.length; i++) {
      const val = prop.enum[i]
      if (val.trim() === '') {
        errors.push({ path: `${path}.enum[${i}]`, message: 'Enum values cannot be empty' })
      } else if (seen.has(val)) {
        errors.push({ path: `${path}.enum[${i}]`, message: `Duplicate enum value: "${val}"` })
      } else {
        seen.add(val)
      }
    }
  }

  // Validate nested properties
  if (prop.type === 'object' && prop.properties) {
    const nestedNames = new Set<string>()
    for (const nestedProp of prop.properties) {
      if (nestedNames.has(nestedProp.name)) {
        errors.push({ path: `${path}.properties.${nestedProp.name}`, message: `Duplicate property name: "${nestedProp.name}"` })
      }
      nestedNames.add(nestedProp.name)
      validateEditableProperty(nestedProp, `${path}.properties.${nestedProp.name}`, errors)
    }
  }

  // Validate array items
  if (prop.type === 'array') {
    if (!prop.items) {
      errors.push({ path: `${path}.items`, message: 'Array type must have items defined' })
    } else {
      validateEditableProperty(prop.items, `${path}.items`, errors)
    }
  }
}

/** Export valid types for use in UI components */
export { VALID_TYPES, VALID_INDEX_TYPES }

/**
 * Create a new empty property
 */
export function createEmptyProperty(type: SchemaPropertyType = 'string'): SchemaProperty {
  const property: SchemaProperty = {
    id: generateId(),
    name: '',
    type,
    description: '',
    required: false,
    unique: false,
    translatable: false,
  }

  if (type === 'object') {
    property.properties = []
    property.allowAdditionalProperties = false
  }

  if (type === 'array') {
    property.items = {
      id: generateId(),
      name: 'items',
      type: 'string',
      description: '',
      required: false,
    }
  }

  return property
}
