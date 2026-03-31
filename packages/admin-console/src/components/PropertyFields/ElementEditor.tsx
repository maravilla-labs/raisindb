import { useMemo } from 'react'
import { FileText } from 'lucide-react'
import { useResolvedElementType } from '../../hooks/useResolvedElementType'
import ArchetypeFieldRenderer from '../ArchetypeFieldRenderer'

interface ElementValue {
  element_type: string
  uuid: string
  [key: string]: unknown
}

interface ElementEditorProps {
  name: string
  label: string
  value: ElementValue | undefined
  error?: string
  onChange: (value: unknown) => void
  /** The expected element type name for this field */
  elementType?: string
  repo?: string
  branch?: string
  translationMode?: boolean
  /** Original (source-language) element value for translation reference */
  originalValue?: unknown
  /** Default language code for "Original (en):" hints */
  defaultLanguage?: string
}

export default function ElementEditor({
  name: _name,
  label,
  value,
  error,
  onChange,
  elementType,
  repo,
  branch,
  translationMode,
  originalValue,
  defaultLanguage,
}: ElementEditorProps) {
  const typeName = value?.element_type ?? elementType ?? null
  const { data: resolvedType, loading } = useResolvedElementType(repo, branch, typeName)
  const originalEl = originalValue as ElementValue | undefined

  // Content fields = everything except element_type and uuid
  const contentKeys = useMemo(
    () =>
      value
        ? Object.keys(value).filter((k) => k !== 'element_type' && k !== 'uuid')
        : [],
    [value]
  )

  function ensureValue(): ElementValue {
    if (value) return value
    return {
      element_type: elementType ?? '',
      uuid: crypto.randomUUID(),
    }
  }

  function handleFieldChange(fieldName: string, fieldValue: unknown) {
    const current = ensureValue()
    onChange({ ...current, [fieldName]: fieldValue })
  }

  return (
    <div>
      <label className="flex items-center gap-2 text-sm font-medium text-zinc-300 mb-3">
        <FileText className="w-4 h-4 text-purple-400" />
        {label}
        {typeName && (
          <span className="text-xs text-zinc-500">({typeName})</span>
        )}
      </label>

      <div className="bg-white/5 border border-white/10 rounded-lg p-4 space-y-3">
        {loading && <p className="text-xs text-zinc-500">Loading element schema...</p>}

        {resolvedType?.resolved_fields && resolvedType.resolved_fields.length > 0
          ? resolvedType.resolved_fields.map((fieldSchema: any) => {
              const fieldName = fieldSchema.base?.name ?? fieldSchema.name
              if (!fieldName) return null
              return (
                <ArchetypeFieldRenderer
                  key={fieldName}
                  field={fieldSchema}
                  value={value?.[fieldName]}
                  onChange={(v) => handleFieldChange(fieldName, v)}
                  translationMode={translationMode}
                  originalValue={originalEl?.[fieldName]}
                  defaultLanguage={defaultLanguage}
                  repo={repo}
                  branch={branch}
                />
              )
            })
          : contentKeys.length > 0
            ? contentKeys.map((key) => (
                <div key={key}>
                  <label className="block text-sm font-medium text-zinc-400 mb-1">{key}</label>
                  <input
                    type="text"
                    value={
                      typeof value![key] === 'object'
                        ? JSON.stringify(value![key])
                        : String(value![key] ?? '')
                    }
                    onChange={(e) => {
                      try {
                        handleFieldChange(key, JSON.parse(e.target.value))
                      } catch {
                        handleFieldChange(key, e.target.value)
                      }
                    }}
                    className="w-full px-3 py-1.5 bg-white/10 border border-white/20 rounded text-sm text-white focus:outline-none focus:ring-1 focus:ring-primary-500"
                  />
                </div>
              ))
            : !loading && (
                <p className="text-sm text-zinc-500">
                  {typeName ? 'No fields defined for this element type' : 'No element type specified'}
                </p>
              )}
      </div>

      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}
