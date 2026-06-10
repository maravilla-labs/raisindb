/**
 * Function I/O Panel
 *
 * Shows the linked function's declared input/output schemas inside the
 * flow step properties panel. The input schema drives a per-field
 * arguments editor (values support template expressions like
 * "{{ input.x }}" or "{{ steps.prev.field }}"); the output schema is
 * rendered as copyable template paths for downstream steps.
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { ArrowDownToLine, ArrowUpFromLine, Braces, Check, Copy, Loader2 } from 'lucide-react'
import { getFunction, type FunctionDetails } from '../../../../api/functions'

interface SchemaField {
  name: string
  type: string
  description?: string
  required: boolean
}

/** Extract flat field list from a JSON schema's top-level properties */
function schemaFields(schema: Record<string, unknown> | undefined): SchemaField[] {
  if (!schema || typeof schema !== 'object') return []
  const properties = schema.properties as Record<string, Record<string, unknown>> | undefined
  if (!properties) return []
  const required = new Set(Array.isArray(schema.required) ? (schema.required as string[]) : [])
  return Object.entries(properties).map(([name, def]) => ({
    name,
    type: typeof def?.type === 'string' ? (def.type as string) : 'any',
    description: typeof def?.description === 'string' ? (def.description as string) : undefined,
    required: required.has(name),
  }))
}

/** Render an argument value for display in a text input */
function argToText(value: unknown): string {
  if (value === undefined || value === null) return ''
  if (typeof value === 'string') return value
  return JSON.stringify(value)
}

/**
 * Parse user text back into an argument value. Template expressions and
 * plain text stay strings; for non-string schema types, JSON scalars and
 * structures are stored natively (so a number field gets 5, not "5").
 */
function textToArg(text: string, fieldType: string): unknown {
  if (text === '') return undefined
  const trimmed = text.trim()
  if (trimmed.includes('{{') || trimmed.includes('${')) return text
  if (fieldType !== 'string') {
    try {
      return JSON.parse(trimmed)
    } catch {
      // fall through - keep as string (may still be a REL expression)
    }
  }
  return text
}

interface FunctionIOPanelProps {
  /** Repository id */
  repo: string
  /** Linked function path (raisin:path, e.g. "/reserve-seats") */
  functionPath: string
  /** Step id - used to build steps.<id>.<field> output templates */
  stepId: string
  /** Current step arguments */
  args?: Record<string, unknown>
  /** Persist updated arguments on the step */
  onChangeArguments: (args: Record<string, unknown> | undefined) => void
}

export function FunctionIOPanel({
  repo,
  functionPath,
  stepId,
  args,
  onChangeArguments,
}: FunctionIOPanelProps) {
  const [details, setDetails] = useState<FunctionDetails | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [rawMode, setRawMode] = useState(false)
  const [rawText, setRawText] = useState('')
  const [rawError, setRawError] = useState<string | null>(null)
  const [copiedField, setCopiedField] = useState<string | null>(null)

  const functionName = functionPath.replace(/^\//, '')

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setLoadError(null)
    setDetails(null)
    getFunction(repo, functionName)
      .then((d) => {
        if (!cancelled) setDetails(d)
      })
      .catch((err: unknown) => {
        if (!cancelled) setLoadError(err instanceof Error ? err.message : 'Failed to load function')
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [repo, functionName])

  const inputFields = useMemo(() => schemaFields(details?.input_schema), [details])
  const outputFields = useMemo(() => schemaFields(details?.output_schema), [details])

  const missingRequired = useMemo(
    () =>
      inputFields
        .filter((f) => f.required)
        .filter((f) => args?.[f.name] === undefined || args?.[f.name] === '')
        .map((f) => f.name),
    [inputFields, args]
  )

  const handleFieldChange = useCallback(
    (field: SchemaField, text: string) => {
      const next: Record<string, unknown> = { ...(args || {}) }
      const value = textToArg(text, field.type)
      if (value === undefined) {
        delete next[field.name]
      } else {
        next[field.name] = value
      }
      onChangeArguments(Object.keys(next).length > 0 ? next : undefined)
    },
    [args, onChangeArguments]
  )

  const enterRawMode = useCallback(() => {
    setRawText(args ? JSON.stringify(args, null, 2) : '{}')
    setRawError(null)
    setRawMode(true)
  }, [args])

  const applyRaw = useCallback(
    (text: string) => {
      setRawText(text)
      if (text.trim() === '' || text.trim() === '{}') {
        setRawError(null)
        onChangeArguments(undefined)
        return
      }
      try {
        const parsed = JSON.parse(text)
        if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
          setRawError('Arguments must be a JSON object')
          return
        }
        setRawError(null)
        onChangeArguments(parsed as Record<string, unknown>)
      } catch {
        setRawError('Invalid JSON')
      }
    },
    [onChangeArguments]
  )

  const copyTemplate = useCallback((field: string, template: string) => {
    navigator.clipboard?.writeText(template).catch(() => {})
    setCopiedField(field)
    setTimeout(() => setCopiedField(null), 1500)
  }, [])

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-xs text-gray-500 py-2">
        <Loader2 className="w-3.5 h-3.5 animate-spin" />
        Loading function schema...
      </div>
    )
  }

  if (loadError) {
    return (
      <p className="text-xs text-gray-500 py-2">
        Could not load schema for <span className="text-gray-400">{functionPath}</span>
      </p>
    )
  }

  // Extra arguments not declared in the schema (still shown in raw mode)
  const extraArgs = Object.keys(args || {}).filter(
    (k) => !inputFields.some((f) => f.name === k)
  )

  return (
    <div className="space-y-4">
      {/* Inputs */}
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <label className="flex items-center gap-1.5 text-xs text-gray-500">
            <ArrowDownToLine className="w-3.5 h-3.5" />
            Inputs
            {inputFields.length > 0 && (
              <span className="text-gray-600">({inputFields.length} declared)</span>
            )}
          </label>
          <button
            onClick={() => (rawMode ? setRawMode(false) : enterRawMode())}
            className={`flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] transition-colors ${
              rawMode
                ? 'bg-blue-500/20 text-blue-300'
                : 'text-gray-500 hover:text-gray-300 hover:bg-white/5'
            }`}
            title="Edit raw arguments JSON"
          >
            <Braces className="w-3 h-3" />
            JSON
          </button>
        </div>

        {rawMode || (inputFields.length === 0 && extraArgs.length > 0) ? (
          <div className="space-y-1">
            <textarea
              value={rawMode ? rawText : JSON.stringify(args, null, 2)}
              onChange={(e) => applyRaw(e.target.value)}
              onFocus={() => {
                if (!rawMode) enterRawMode()
              }}
              rows={6}
              spellCheck={false}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-xs font-mono placeholder-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500"
              placeholder={'{\n  "field": "{{ input.value }}"\n}'}
            />
            {rawError && <p className="text-xs text-red-400">{rawError}</p>}
          </div>
        ) : inputFields.length > 0 ? (
          <div className="space-y-2">
            {inputFields.map((field) => (
              <div key={field.name} className="space-y-1">
                <div className="flex items-center gap-1.5">
                  <span className="text-xs text-gray-300 font-mono">{field.name}</span>
                  <span className="text-[10px] text-gray-600">{field.type}</span>
                  {field.required && (
                    <span
                      className={`text-[10px] ${
                        missingRequired.includes(field.name) ? 'text-amber-400' : 'text-gray-600'
                      }`}
                    >
                      required
                    </span>
                  )}
                </div>
                <input
                  type="text"
                  value={argToText(args?.[field.name])}
                  onChange={(e) => handleFieldChange(field, e.target.value)}
                  spellCheck={false}
                  className="w-full px-3 py-1.5 bg-white/5 border border-white/10 rounded-lg text-white text-xs font-mono placeholder-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500"
                  placeholder={`{{ input.${field.name} }}`}
                  title={field.description}
                />
                {field.description && (
                  <p className="text-[10px] text-gray-600">{field.description}</p>
                )}
              </div>
            ))}
          </div>
        ) : (
          <p className="text-xs text-gray-600 italic">
            No input schema declared - use JSON to pass arguments.
          </p>
        )}

        {missingRequired.length > 0 && !rawMode && (
          <p className="text-xs text-amber-400">
            Missing required: {missingRequired.join(', ')}
          </p>
        )}
      </div>

      {/* Outputs */}
      {outputFields.length > 0 && (
        <div className="space-y-2">
          <label className="flex items-center gap-1.5 text-xs text-gray-500">
            <ArrowUpFromLine className="w-3.5 h-3.5" />
            Outputs
            <span className="text-gray-600">- click to copy template</span>
          </label>
          <div className="space-y-1">
            {outputFields.map((field) => {
              const template = `{{ steps.${stepId}.${field.name} }}`
              return (
                <button
                  key={field.name}
                  onClick={() => copyTemplate(field.name, template)}
                  className="w-full flex items-center gap-2 px-2.5 py-1.5 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 transition-colors text-left group"
                  title={field.description || template}
                >
                  <span className="text-xs text-gray-300 font-mono flex-1 truncate">
                    {field.name}
                  </span>
                  <span className="text-[10px] text-gray-600">{field.type}</span>
                  {copiedField === field.name ? (
                    <Check className="w-3 h-3 text-green-400" />
                  ) : (
                    <Copy className="w-3 h-3 text-gray-600 group-hover:text-gray-400" />
                  )}
                </button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
