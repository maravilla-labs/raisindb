export function getSchemaType(schema: any): string {
  if (!schema) return 'string'
  const rawType =
    schema.type ??
    schema.property_type ??
    schema.propertyType ??
    (typeof schema.property_type === 'object' ? schema.property_type?.type : undefined) ??
    (typeof schema.propertyType === 'object' ? schema.propertyType?.type : undefined)

  if (typeof rawType === 'string') {
    return rawType.toLowerCase()
  }

  if (rawType && typeof rawType === 'object' && typeof rawType.type === 'string') {
    return rawType.type.toLowerCase()
  }

  return 'string'
}

export function getSchemaLabel(name: string, schema: any): string {
  return schema?.label || schema?.title || formatLabel(name)
}

export function getSchemaPlaceholder(schema: any): string | undefined {
  return schema?.placeholder || schema?.hint || schema?.descriptionPlaceholder
}

export function getSchemaDescription(schema: any): string | undefined {
  return schema?.description || schema?.help_text || schema?.helpText
}

export function getSchemaStructure(schema: any): Record<string, any> | undefined {
  return schema?.structure || schema?.properties
}

export function getSchemaItems(schema: any): any {
  return schema?.items || schema?.item
}

export function getSchemaEnum(schema: any): Array<string | { value: string; label?: string }> | undefined {
  if (Array.isArray(schema?.enum)) return schema.enum
  if (Array.isArray(schema?.options)) return schema.options
  if (Array.isArray(schema?.values)) return schema.values
  if (Array.isArray(schema?.allowed)) return schema.allowed
  return undefined
}

export function isSchemaRequired(schema: any): boolean {
  return Boolean(schema?.required)
}

export function isSchemaTranslatable(schema: any): boolean {
  if (schema?.translatable !== undefined) return Boolean(schema.translatable)
  if (schema?.is_translatable !== undefined) return Boolean(schema.is_translatable)
  return false
}

export function getDefaultValueForSchema(schema: any): any {
  if (schema?.default !== undefined) {
    return schema.default
  }

  switch (getSchemaType(schema)) {
    case 'boolean':
      return false
    case 'array':
      return []
    case 'object':
      return {}
    case 'date':
      return ''
    case 'number':
    case 'integer':
      return undefined
    case 'composite':
    case 'element':
      return undefined
    case 'resource':
    case 'reference':
    case 'string':
    default:
      return ''
  }
}

export function validateValueAgainstSchema(
  name: string,
  value: any,
  schema: any
): string | null {
  const label = getSchemaLabel(name, schema)
  const type = getSchemaType(schema)
  const required = isSchemaRequired(schema)

  if (required && (value === undefined || value === null || value === '')) {
    return `${label} is required`
  }

  if (value === undefined || value === null || value === '') {
    return null
  }

  switch (type) {
    case 'string': {
      if (typeof value !== 'string') {
        return `${label} must be a string`
      }
      if (schema.minLength && value.length < schema.minLength) {
        return `${label} must be at least ${schema.minLength} characters`
      }
      if (schema.maxLength && value.length > schema.maxLength) {
        return `${label} must be at most ${schema.maxLength} characters`
      }
      if (schema.pattern) {
        try {
          const regex = new RegExp(schema.pattern)
          if (!regex.test(value)) {
            return `${label} has an invalid format`
          }
        } catch {
          // ignore invalid regex in schema
        }
      }
      break
    }
    case 'number':
    case 'integer': {
      if (typeof value !== 'number' || Number.isNaN(value)) {
        return `${label} must be a number`
      }
      if (schema.minimum !== undefined && value < schema.minimum) {
        return `${label} must be at least ${schema.minimum}`
      }
      if (schema.maximum !== undefined && value > schema.maximum) {
        return `${label} must be at most ${schema.maximum}`
      }
      if (type === 'integer' && !Number.isInteger(value)) {
        return `${label} must be an integer`
      }
      break
    }
    case 'boolean': {
      if (typeof value !== 'boolean') {
        return `${label} must be a boolean`
      }
      break
    }
    case 'array': {
      if (!Array.isArray(value)) {
        return `${label} must be an array`
      }
      if (schema.minItems !== undefined && value.length < schema.minItems) {
        return `${label} must have at least ${schema.minItems} items`
      }
      if (schema.maxItems !== undefined && value.length > schema.maxItems) {
        return `${label} must have at most ${schema.maxItems} items`
      }
      break
    }
    case 'object': {
      if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        return `${label} must be an object`
      }
      break
    }
    case 'date': {
      if (typeof value !== 'string') {
        return `${label} must be a date string`
      }
      if (Number.isNaN(Date.parse(value))) {
        return `${label} must be a valid date`
      }
      break
    }
    default:
      break
  }

  const enumValues = getSchemaEnum(schema)
  if (enumValues && enumValues.length > 0) {
    const allowed = enumValues.map((opt: any) =>
      typeof opt === 'string' ? opt : opt.value
    )
    if (!allowed.includes(value)) {
      return `${label} must be one of: ${allowed.join(', ')}`
    }
  }

  return null
}

export function formatLabel(name: string): string {
  return name
    .replace(/_/g, ' ')
    .replace(/-/g, ' ')
    .split(' ')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ')
}

export function createDefaultFromSchema(schema: any): any {
  const defaultValue = getDefaultValueForSchema(schema)
  if (defaultValue !== undefined) {
    return defaultValue
  }
  return undefined
}

export function getEnumOptions(schema: any): { value: string; label: string }[] | undefined {
  const enumValues = getSchemaEnum(schema)
  if (!enumValues) return undefined
  return enumValues.map((opt: any) =>
    typeof opt === 'string'
      ? { value: opt, label: opt }
      : { value: String(opt.value), label: opt.label ?? String(opt.value) }
  )
}
