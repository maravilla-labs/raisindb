import * as yaml from 'js-yaml'
import { nanoid } from 'nanoid'
import type { NodeTypeDefinition, PropertyValueSchema } from './types'

/**
 * Parse YAML string to NodeTypeDefinition with internal IDs
 */
export function parseYamlToNodeType(yamlContent: string): NodeTypeDefinition {
  const parsed = yaml.load(yamlContent) as any

  // Ensure properties array exists and add IDs
  const properties: PropertyValueSchema[] = Array.isArray(parsed.properties)
    ? parsed.properties.map((prop: any) => addPropertyIds(prop))
    : []

  return {
    id: parsed.id,
    name: parsed.name || '',
    extends: parsed.extends,
    icon: parsed.icon,
    description: parsed.description,
    version: parsed.version,
    strict: parsed.strict,
    versionable: parsed.versionable,
    publishable: parsed.publishable,
    auditable: parsed.auditable,
    indexable: parsed.indexable,
    index_types: parsed.index_types,
    allowed_children: Array.isArray(parsed.allowed_children) ? parsed.allowed_children : [],
    required_nodes: Array.isArray(parsed.required_nodes) ? parsed.required_nodes : [],
    properties,
    mixins: parsed.mixins,
    overrides: parsed.overrides,
    initial_structure: parsed.initial_structure,
    compound_indexes: Array.isArray(parsed.compound_indexes) ? parsed.compound_indexes : undefined,
  }
}

/**
 * Add internal IDs to properties for React keys and drag-drop
 */
function addPropertyIds(property: any): PropertyValueSchema {
  const prop: PropertyValueSchema = {
    ...property,
    id: property.id || nanoid(),
  }

  if (prop.translatable === undefined && prop.is_translatable !== undefined) {
    prop.translatable = prop.is_translatable
    delete (prop as any).is_translatable
  }

  if (prop.meta && typeof prop.meta === 'object' && !Array.isArray(prop.meta)) {
    const meta = { ...(prop.meta as Record<string, any>) }
    if (prop.label === undefined && typeof meta.label === 'string') {
      prop.label = meta.label
      delete meta.label
    }
    if (prop.description === undefined && typeof meta.description === 'string') {
      prop.description = meta.description
      delete meta.description
    }
    if (prop.placeholder === undefined && typeof meta.placeholder === 'string') {
      prop.placeholder = meta.placeholder
      delete meta.placeholder
    }
    if (prop.multiline === undefined && typeof meta.multiline === 'boolean') {
      prop.multiline = meta.multiline
      delete meta.multiline
    }

    const metaEnum = meta.enum ?? meta.options ?? meta.values
    if (prop.enum === undefined && Array.isArray(metaEnum)) {
      prop.enum = metaEnum
      delete meta.enum
      delete meta.options
      delete meta.values
    }

    if (Object.keys(meta).length > 0) {
      prop.meta = meta
    } else {
      delete (prop as any).meta
    }
  }

  if (!prop.enum) {
    const altEnum = (prop as any).options ?? (prop as any).values
    if (Array.isArray(altEnum)) {
      prop.enum = altEnum
    }
  }
  if ((prop as any).options !== undefined) {
    delete (prop as any).options
  }
  if ((prop as any).values !== undefined) {
    delete (prop as any).values
  }

  // Recursively add IDs to nested structures
  // items is a single schema for Array type
  if (prop.items && !Array.isArray(prop.items)) {
    prop.items = addPropertyIds(prop.items)
  }

  // fields is an array of schemas for Composite type
  if (prop.fields && Array.isArray(prop.fields)) {
    prop.fields = prop.fields.map((field: any) => addPropertyIds(field))
  }

  if (prop.structure) {
    prop.structure = Object.fromEntries(
      Object.entries(prop.structure).map(([key, value]) => [
        key,
        addPropertyIds(value as any),
      ])
    )
  }

  return prop
}

/**
 * Remove internal IDs before serialization
 */
function cleanPropertyIds(property: PropertyValueSchema): any {
  const { id, ...rest } = property
  const cleaned: any = { ...rest }

  if (cleaned.is_translatable !== undefined) {
    delete cleaned.is_translatable
  }
  if (cleaned.translatable !== undefined) {
    cleaned.translatable = Boolean(cleaned.translatable)
  }
  if (cleaned.options !== undefined) {
    delete cleaned.options
  }
  if (cleaned.values !== undefined) {
    delete cleaned.values
  }

  // Recursively clean nested structures
  // items is a single schema for Array type
  if (cleaned.items && !Array.isArray(cleaned.items)) {
    cleaned.items = cleanPropertyIds(cleaned.items)
  }

  // fields is an array of schemas for Composite type
  if (cleaned.fields && Array.isArray(cleaned.fields)) {
    cleaned.fields = cleaned.fields.map((field: any) => cleanPropertyIds(field))
  }

  if (cleaned.structure) {
    cleaned.structure = Object.fromEntries(
      Object.entries(cleaned.structure).map(([key, value]) => [
        key,
        cleanPropertyIds(value as PropertyValueSchema),
      ])
    )
  }

  return cleaned
}

/**
 * Serialize NodeTypeDefinition to YAML string
 */
export function serializeNodeTypeToYaml(nodeType: NodeTypeDefinition): string {
  // Clean up the object for YAML export
  const cleaned: any = {
    name: nodeType.name,
  }

  if (nodeType.extends) cleaned.extends = nodeType.extends
  if (nodeType.description) cleaned.description = nodeType.description
  if (nodeType.icon) cleaned.icon = nodeType.icon
  if (nodeType.version !== undefined) cleaned.version = nodeType.version
  if (nodeType.strict !== undefined) cleaned.strict = nodeType.strict
  if (nodeType.indexable !== undefined) cleaned.indexable = nodeType.indexable
  if (nodeType.index_types && nodeType.index_types.length > 0) {
    cleaned.index_types = nodeType.index_types
  }

  // Only include properties if there are any
  if (nodeType.properties && nodeType.properties.length > 0) {
    cleaned.properties = nodeType.properties.map(prop => cleanPropertyIds(prop))
  }

  if (nodeType.allowed_children && nodeType.allowed_children.length > 0) {
    cleaned.allowed_children = nodeType.allowed_children
  }

  if (nodeType.required_nodes && nodeType.required_nodes.length > 0) {
    cleaned.required_nodes = nodeType.required_nodes
  }

  if (nodeType.mixins && nodeType.mixins.length > 0) {
    cleaned.mixins = nodeType.mixins
  }

  if (nodeType.overrides) cleaned.overrides = nodeType.overrides
  if (nodeType.initial_structure) cleaned.initial_structure = nodeType.initial_structure
  if (nodeType.versionable !== undefined) cleaned.versionable = nodeType.versionable
  if (nodeType.publishable !== undefined) cleaned.publishable = nodeType.publishable
  if (nodeType.auditable !== undefined) cleaned.auditable = nodeType.auditable

  // Include compound indexes if defined
  if (nodeType.compound_indexes && nodeType.compound_indexes.length > 0) {
    cleaned.compound_indexes = nodeType.compound_indexes
  }

  return yaml.dump(cleaned, {
    indent: 2,
    lineWidth: -1, // Don't wrap lines
    noRefs: true,
    sortKeys: false,
  })
}

/**
 * Create a new property with default values
 */
export function createNewProperty(type: import('./types').PropertyType): PropertyValueSchema {
  const base: PropertyValueSchema = {
    id: nanoid(),
    name: `new_${type.toLowerCase()}_${nanoid(4)}`,
    type,
    required: false,
    translatable:
      type === 'String' ||
      type === 'Element' ||
      type === 'Composite' ||
      type === 'Object',
  }

  // Add type-specific defaults
  switch (type) {
    case 'Array':
      base.items = {
        id: nanoid(),
        type: 'String',
      }
      break
    case 'Object':
      base.structure = {}
      base.allow_additional_properties = false
      break
    case 'Composite':
      base.fields = [] // Composite uses fields array (ordered, sortable)
      break
    case 'Boolean':
      base.default = false
      break
    case 'Number':
      base.default = 0
      break
    case 'Date':
      base.default = ''
      break
    case 'Reference':
    case 'NodeType':
    case 'URL':
    case 'Resource':
      base.default = ''
      break
  }

  return base
}

/**
 * Validate node type definition
 */
export function validateNodeType(nodeType: NodeTypeDefinition): Record<string, string> {
  const errors: Record<string, string> = {}

  // Name validation
  if (!nodeType.name) {
    errors.name = 'Node type name is required'
  } else if (!/^[a-z][a-z0-9]*:[A-Z][a-zA-Z0-9]*$/.test(nodeType.name)) {
    errors.name = 'Name must be in format "namespace:TypeName" (e.g., "raisin:Page")'
  }

  // Extends validation
  if (nodeType.extends && !/^[a-z][a-z0-9]*:[A-Z][a-zA-Z0-9]*$/.test(nodeType.extends)) {
    errors.extends = 'Extends must be in format "namespace:TypeName"'
  }

  // Property validation
  nodeType.properties.forEach((prop, index) => {
    if (!prop.name) {
      errors[`property_${index}_name`] = 'Property name is required'
    } else if (!/^[a-z_][a-z0-9_]*$/.test(prop.name)) {
      errors[`property_${index}_name`] = 'Property name must be lowercase with underscores'
    }

    // Check for duplicate property names
    const duplicates = nodeType.properties.filter(p => p.name === prop.name)
    if (duplicates.length > 1) {
      errors[`property_${index}_name`] = 'Duplicate property name'
    }
  })

  return errors
}
