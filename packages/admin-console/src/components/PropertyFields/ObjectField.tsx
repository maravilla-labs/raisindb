import { useState } from 'react'
import { ChevronRight, ChevronDown, Braces } from 'lucide-react'

interface ObjectFieldProps {
  name: string
  label: string
  value: Record<string, any> | undefined
  error?: string
  required?: boolean
  schema?: Record<string, any>
  onChange: (value: Record<string, any>) => void
}

export default function ObjectField({
  name: _name,
  label,
  value,
  error,
  required,
  schema,
  onChange
}: ObjectFieldProps) {
  const [expanded, setExpanded] = useState(true)
  const [jsonMode, setJsonMode] = useState(false)
  const [jsonInput, setJsonInput] = useState('')

  const handleJsonEdit = (text: string) => {
    setJsonInput(text)
    try {
      const parsed = JSON.parse(text)
      onChange(parsed)
    } catch {
      // Invalid JSON, don't update
    }
  }

  const toggleJsonMode = () => {
    if (!jsonMode) {
      setJsonInput(JSON.stringify(value || {}, null, 2))
    }
    setJsonMode(!jsonMode)
  }

  const handleFieldChange = (fieldName: string, fieldValue: any) => {
    onChange({
      ...value,
      [fieldName]: fieldValue
    })
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-2">
        <label className="flex items-center gap-2 text-sm font-medium text-zinc-300">
          <button
            type="button"
            onClick={() => setExpanded(!expanded)}
            className="p-0.5 hover:bg-white/10 rounded"
          >
            {expanded ? (
              <ChevronDown className="w-4 h-4" />
            ) : (
              <ChevronRight className="w-4 h-4" />
            )}
          </button>
          <Braces className="w-4 h-4 text-orange-400" />
          {label}
          {required && <span className="text-red-400">*</span>}
        </label>
        <button
          type="button"
          onClick={toggleJsonMode}
          className="text-xs text-primary-400 hover:text-primary-300"
        >
          {jsonMode ? 'Form View' : 'JSON View'}
        </button>
      </div>

      {expanded && (
        <div className="ml-6 p-4 bg-white/5 rounded-lg">
          {jsonMode ? (
            <textarea
              value={jsonInput}
              onChange={(e) => handleJsonEdit(e.target.value)}
              className={`w-full px-4 py-2 bg-white/10 border rounded-lg text-white font-mono text-sm focus:outline-none focus:ring-2 ${
                error
                  ? 'border-red-500/50 focus:ring-red-500'
                  : 'border-white/20 focus:ring-primary-500'
              }`}
              rows={8}
            />
          ) : (
            <div className="space-y-3">
              {schema ? (
                Object.entries(schema).map(([fieldName, fieldSchema]: [string, any]) => (
                  <div key={fieldName}>
                    <label className="block text-sm text-zinc-400 mb-1">
                      {fieldSchema.label || fieldName}
                      {fieldSchema.required && <span className="text-red-400 ml-1">*</span>}
                    </label>
                    <input
                      type="text"
                      value={value?.[fieldName] || ''}
                      onChange={(e) => handleFieldChange(fieldName, e.target.value)}
                      className="w-full px-3 py-1.5 bg-white/10 border border-white/20 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-500"
                    />
                  </div>
                ))
              ) : (
                <div className="text-sm text-zinc-500">
                  No schema defined. Use JSON view to edit.
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}