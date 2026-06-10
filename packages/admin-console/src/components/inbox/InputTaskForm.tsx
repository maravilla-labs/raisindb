/**
 * Input task form
 *
 * Minimal JSON-schema form renderer for input tasks. Supports a top-level
 * object schema with string (text / textarea), number, integer, boolean and
 * enum properties. No external form library. Reusable outside the inbox page.
 */

import { useMemo, useState } from 'react'

/** Subset of JSON schema we render */
interface JsonSchemaProperty {
  type?: string
  title?: string
  description?: string
  format?: string
  enum?: (string | number)[]
  default?: unknown
}

interface JsonSchemaObject {
  type?: string
  properties?: Record<string, JsonSchemaProperty>
  required?: string[]
}

interface InputTaskFormProps {
  schema: Record<string, unknown>
  onSubmit: (values: Record<string, unknown>) => void
  busy?: boolean
}

const FIELD_CLASS =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-zinc-300 placeholder-zinc-500 focus:outline-none focus:border-purple-500 disabled:opacity-50'

/** Use a textarea for explicit textarea format or long descriptions */
function isTextarea(prop: JsonSchemaProperty): boolean {
  return prop.format === 'textarea' || (prop.description?.length || 0) > 80
}

export default function InputTaskForm({ schema, onSubmit, busy }: InputTaskFormProps) {
  const objectSchema = schema as JsonSchemaObject
  const properties = objectSchema.properties || {}
  const required = useMemo(() => new Set(objectSchema.required || []), [objectSchema.required])

  const [values, setValues] = useState<Record<string, unknown>>(() => {
    const initial: Record<string, unknown> = {}
    for (const [key, prop] of Object.entries(properties)) {
      if (prop.default !== undefined) initial[key] = prop.default
      else if (prop.type === 'boolean') initial[key] = false
    }
    return initial
  })
  const [errors, setErrors] = useState<Record<string, string>>({})

  const setValue = (key: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [key]: value }))
    setErrors((prev) => {
      if (!prev[key]) return prev
      const next = { ...prev }
      delete next[key]
      return next
    })
  }

  const handleSubmit = () => {
    if (busy) return

    // Validate required fields and coerce numbers
    const result: Record<string, unknown> = {}
    const newErrors: Record<string, string> = {}

    for (const [key, prop] of Object.entries(properties)) {
      const raw = values[key]
      const isEmpty = raw === undefined || raw === null || raw === ''

      if (isEmpty) {
        if (required.has(key) && prop.type !== 'boolean') {
          newErrors[key] = 'Required'
        }
        continue
      }

      if (prop.type === 'number' || prop.type === 'integer') {
        const num = Number(raw)
        if (Number.isNaN(num)) {
          newErrors[key] = 'Must be a number'
          continue
        }
        if (prop.type === 'integer' && !Number.isInteger(num)) {
          newErrors[key] = 'Must be an integer'
          continue
        }
        result[key] = num
      } else {
        result[key] = raw
      }
    }

    if (Object.keys(newErrors).length > 0) {
      setErrors(newErrors)
      return
    }

    onSubmit(result)
  }

  const fieldKeys = Object.keys(properties)

  return (
    <div className="space-y-3">
      {fieldKeys.length === 0 && (
        <p className="text-sm text-zinc-500">No input fields defined.</p>
      )}

      {fieldKeys.map((key) => {
        const prop = properties[key]
        const label = prop.title || key
        const isRequired = required.has(key)

        return (
          <div key={key}>
            <label className="block text-xs text-zinc-500 mb-1.5">
              {label}
              {isRequired && <span className="text-red-400 ml-1">*</span>}
            </label>

            {prop.enum ? (
              <select
                value={String(values[key] ?? '')}
                onChange={(e) => {
                  // Map back to the original enum value (preserve numbers)
                  const match = prop.enum?.find((v) => String(v) === e.target.value)
                  setValue(key, e.target.value === '' ? undefined : match ?? e.target.value)
                }}
                disabled={busy}
                className={FIELD_CLASS}
              >
                <option value="">Select...</option>
                {prop.enum.map((option) => (
                  <option key={String(option)} value={String(option)}>
                    {String(option)}
                  </option>
                ))}
              </select>
            ) : prop.type === 'boolean' ? (
              <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer">
                <input
                  type="checkbox"
                  checked={Boolean(values[key])}
                  onChange={(e) => setValue(key, e.target.checked)}
                  disabled={busy}
                  className="w-4 h-4 rounded border-white/20 bg-white/5 accent-purple-500"
                />
                {prop.description || label}
              </label>
            ) : prop.type === 'number' || prop.type === 'integer' ? (
              <input
                type="number"
                step={prop.type === 'integer' ? 1 : 'any'}
                value={values[key] === undefined ? '' : String(values[key])}
                onChange={(e) => setValue(key, e.target.value)}
                placeholder={prop.description}
                disabled={busy}
                className={FIELD_CLASS}
              />
            ) : isTextarea(prop) ? (
              <textarea
                value={String(values[key] ?? '')}
                onChange={(e) => setValue(key, e.target.value)}
                rows={3}
                placeholder={prop.description}
                disabled={busy}
                className={FIELD_CLASS}
              />
            ) : (
              <input
                type="text"
                value={String(values[key] ?? '')}
                onChange={(e) => setValue(key, e.target.value)}
                placeholder={prop.description}
                disabled={busy}
                className={FIELD_CLASS}
              />
            )}

            {prop.description && prop.type !== 'boolean' && !isTextarea(prop) && prop.enum === undefined && (
              <p className="text-xs text-zinc-600 mt-1">{prop.description}</p>
            )}
            {errors[key] && <p className="text-xs text-red-400 mt-1">{errors[key]}</p>}
          </div>
        )
      })}

      <button
        onClick={handleSubmit}
        disabled={busy}
        className="px-4 py-2 bg-purple-500 hover:bg-purple-600 rounded-lg text-sm font-medium text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {busy ? 'Submitting...' : 'Submit'}
      </button>
    </div>
  )
}
