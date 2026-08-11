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

/**
 * UI hints authored on a NodeType arrive under `meta`.
 *
 * The server's `PropertyValueSchema` has no `title`, `description`, `enum`,
 * `label` or `placeholder` field and no catch-all, so serde silently DROPS
 * those keys: they never reach `resolved_properties`. `meta` is the only
 * free-form map that survives the round trip, which is why schema authors are
 * told to put presentation hints there. These helpers therefore check the
 * top-level key first (for schemas supplied inline by the client, where it does
 * survive) and fall back to `meta`.
 */
function metaOf(schema: any): Record<string, any> {
  return (schema?.meta as Record<string, any>) || {}
}

export function getSchemaLabel(name: string, schema: any): string {
  const meta = metaOf(schema)
  return schema?.label || schema?.title || meta.label || meta.title || formatLabel(name)
}

export function getSchemaPlaceholder(schema: any): string | undefined {
  const meta = metaOf(schema)
  return (
    schema?.placeholder || schema?.hint || schema?.descriptionPlaceholder || meta.placeholder
  )
}

export function getSchemaDescription(schema: any): string | undefined {
  const meta = metaOf(schema)
  return schema?.description || schema?.help_text || schema?.helpText || meta.description
}

/**
 * Whether this property holds a secret.
 *
 * Secret values are never sent to the browser — only the field NAME appears, in
 * `secret_fields` / `config_secret_fields`. The form renders a write-only input
 * and submits changes through the dedicated encrypting endpoints.
 *
 * `encrypted` is the first-class spelling: `PropertyValueSchema` and
 * `FieldTypeSchema` both carry `encrypted: Option<bool>`, and the server's write
 * layer vaults such a field into the secret store. It is checked FIRST because
 * it is the one the server acts on — a form that rendered a plain input for it
 * would show the operator a `secret://…` reference where a value belongs.
 *
 * The two legacy spellings stay honoured, matching the server's own `is_secret`:
 * a top-level `secret` (client-supplied inline schemas) and `meta.secret`
 * (already-shipped authored schemas, where free-form `meta` was the only key
 * that survived the serde round trip).
 */
export function isSecretField(schema: any): boolean {
  return Boolean(schema?.encrypted ?? schema?.secret ?? metaOf(schema).encrypted ?? metaOf(schema).secret)
}

/** Optional grouping key, so related fields render together. */
export function getFieldGroup(schema: any): string | undefined {
  return schema?.group || metaOf(schema).group
}

/** Sort order within a group. Unordered fields sink to the bottom. */
export function getFieldOrder(schema: any): number {
  const raw = schema?.order ?? metaOf(schema).order
  const n = Number(raw)
  return Number.isFinite(n) ? n : Number.MAX_SAFE_INTEGER
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
  // Server-resolved schemas carry it here — see metaOf().
  const meta = metaOf(schema)
  if (Array.isArray(meta.enum)) return meta.enum
  if (Array.isArray(meta.options)) return meta.options
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
