/**
 * Schema Code Editor Component
 *
 * Monaco-based JSON editor for direct JSON Schema editing with validation.
 */

import { useCallback, useEffect, useState } from 'react'
import Editor from '@monaco-editor/react'
import { AlertCircle } from 'lucide-react'
import { validateSchema, type SchemaValidationError } from './schema-types'

interface SchemaCodeEditorProps {
  /** Current schema as JSON string */
  value: string
  /** Called when the JSON changes (even if invalid) */
  onChange: (value: string) => void
  /** Called when the JSON is parsed successfully */
  onSchemaChange?: (schema: Record<string, unknown>) => void
  /** Validation errors from parent */
  errors?: SchemaValidationError[]
}

export function SchemaCodeEditor({
  value,
  onChange,
  onSchemaChange,
  errors: externalErrors = [],
}: SchemaCodeEditorProps) {
  const [syntaxError, setSyntaxError] = useState<string | null>(null)
  const [validationErrors, setValidationErrors] = useState<SchemaValidationError[]>([])

  // Validate JSON on change
  useEffect(() => {
    setSyntaxError(null)
    setValidationErrors([])

    if (!value.trim()) {
      return
    }

    try {
      const parsed = JSON.parse(value)

      // Validate schema structure
      const schemaErrors = validateSchema(parsed)
      setValidationErrors(schemaErrors)

      // Notify parent of valid schema
      if (schemaErrors.length === 0 && onSchemaChange) {
        onSchemaChange(parsed)
      }
    } catch (e) {
      setSyntaxError((e as Error).message)
    }
  }, [value, onSchemaChange])

  const handleEditorChange = useCallback(
    (newValue: string | undefined) => {
      onChange(newValue || '')
    },
    [onChange]
  )

  const allErrors = [
    ...(syntaxError ? [{ path: '', message: `JSON syntax error: ${syntaxError}` }] : []),
    ...validationErrors,
    ...externalErrors,
  ]

  return (
    <div className="flex flex-col h-full">
      {/* Editor */}
      <div className="flex-1 min-h-0 border border-white/10 rounded-lg overflow-hidden">
        <Editor
          height="100%"
          language="json"
          theme="vs-dark"
          value={value}
          onChange={handleEditorChange}
          options={{
            minimap: { enabled: false },
            fontSize: 13,
            lineNumbers: 'on',
            wordWrap: 'on',
            scrollBeyondLastLine: false,
            padding: { top: 8, bottom: 8 },
            automaticLayout: true,
            tabSize: 2,
            folding: true,
            foldingHighlight: true,
            bracketPairColorization: { enabled: true },
            formatOnPaste: true,
            formatOnType: true,
          }}
        />
      </div>

      {/* Validation Errors */}
      {allErrors.length > 0 && (
        <div className="mt-3 p-3 bg-red-500/10 border border-red-500/30 rounded-lg">
          <div className="flex items-center gap-2 text-red-400 text-sm font-medium mb-2">
            <AlertCircle className="w-4 h-4" />
            Validation Errors ({allErrors.length})
          </div>
          <ul className="space-y-1 text-sm text-red-300">
            {allErrors.slice(0, 5).map((error, i) => (
              <li key={i} className="flex gap-2">
                <span className="text-red-500/70 shrink-0">
                  {error.path || 'root'}:
                </span>
                <span>{error.message}</span>
              </li>
            ))}
            {allErrors.length > 5 && (
              <li className="text-red-400/70">
                ... and {allErrors.length - 5} more errors
              </li>
            )}
          </ul>
        </div>
      )}

      {/* Help text */}
      <div className="mt-3 text-xs text-gray-500">
        <p>
          Enter a valid JSON Schema object. The schema must have{' '}
          <code className="bg-white/5 px-1 rounded">type: "object"</code> at the root.
        </p>
      </div>
    </div>
  )
}
