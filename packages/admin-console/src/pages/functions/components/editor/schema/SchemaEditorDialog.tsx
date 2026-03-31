/**
 * Schema Editor Dialog Component
 *
 * Modal dialog for editing JSON Schema with dual-mode support:
 * - Visual mode: Form-based property builder
 * - JSON mode: Monaco code editor
 */

import { useState, useCallback, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { X, Braces, Code2, Eye } from 'lucide-react'
import { SchemaVisualBuilder } from './SchemaVisualBuilder'
import { SchemaCodeEditor } from './SchemaCodeEditor'
import {
  type EditableSchema,
  type SchemaValidationError,
  schemaToEditable,
  editableToSchema,
  validateEditableSchema,
  validateSchema,
} from './schema-types'

type EditorMode = 'visual' | 'json'

interface SchemaEditorDialogProps {
  /** Dialog title (e.g., "Input Schema" or "Output Schema") */
  title: string
  /** Current schema value */
  schema: Record<string, unknown> | undefined
  /** Called when the schema is saved */
  onSave: (schema: Record<string, unknown>) => void
  /** Called when the dialog is closed */
  onClose: () => void
}

export function SchemaEditorDialog({
  title,
  schema,
  onSave,
  onClose,
}: SchemaEditorDialogProps) {
  // Editor mode state
  const [mode, setMode] = useState<EditorMode>('visual')

  // Visual mode state
  const [editableSchema, setEditableSchema] = useState<EditableSchema>(() =>
    schemaToEditable(schema)
  )

  // JSON mode state
  const [jsonText, setJsonText] = useState<string>(() =>
    JSON.stringify(schema || { type: 'object', properties: {} }, null, 2)
  )

  // Validation errors
  const [errors, setErrors] = useState<SchemaValidationError[]>([])

  // Track if we have unsaved changes
  const [hasChanges, setHasChanges] = useState(false)

  // Validate when editable schema changes
  useEffect(() => {
    if (mode === 'visual') {
      const validationErrors = validateEditableSchema(editableSchema)
      setErrors(validationErrors)
    }
  }, [editableSchema, mode])

  // Validate when JSON text changes
  useEffect(() => {
    if (mode === 'json') {
      try {
        const parsed = JSON.parse(jsonText)
        const validationErrors = validateSchema(parsed)
        setErrors(validationErrors)
      } catch {
        // Syntax errors are handled by SchemaCodeEditor
        setErrors([])
      }
    }
  }, [jsonText, mode])

  // Switch to visual mode
  const switchToVisual = useCallback(() => {
    try {
      const parsed = JSON.parse(jsonText)
      const converted = schemaToEditable(parsed)
      setEditableSchema(converted)
      setMode('visual')
    } catch (e) {
      // If JSON is invalid, show error and don't switch
      setErrors([{ path: '', message: `Cannot switch: ${(e as Error).message}` }])
    }
  }, [jsonText])

  // Switch to JSON mode
  const switchToJson = useCallback(() => {
    const schemaObj = editableToSchema(editableSchema)
    setJsonText(JSON.stringify(schemaObj, null, 2))
    setMode('json')
  }, [editableSchema])

  // Handle visual schema changes
  const handleVisualChange = useCallback((updated: EditableSchema) => {
    setEditableSchema(updated)
    setHasChanges(true)
  }, [])

  // Handle JSON text changes
  const handleJsonChange = useCallback((text: string) => {
    setJsonText(text)
    setHasChanges(true)
  }, [])

  // Handle save
  const handleSave = useCallback(() => {
    let schemaToSave: Record<string, unknown>

    if (mode === 'visual') {
      // Validate before saving
      const validationErrors = validateEditableSchema(editableSchema)
      if (validationErrors.length > 0) {
        setErrors(validationErrors)
        return
      }
      schemaToSave = editableToSchema(editableSchema)
    } else {
      // Parse and validate JSON
      try {
        schemaToSave = JSON.parse(jsonText)
        const validationErrors = validateSchema(schemaToSave)
        if (validationErrors.length > 0) {
          setErrors(validationErrors)
          return
        }
      } catch (e) {
        setErrors([{ path: '', message: `Invalid JSON: ${(e as Error).message}` }])
        return
      }
    }

    onSave(schemaToSave)
  }, [mode, editableSchema, jsonText, onSave])

  // Handle close with confirmation
  const handleClose = useCallback(() => {
    if (hasChanges) {
      const confirmed = window.confirm('You have unsaved changes. Discard them?')
      if (!confirmed) return
    }
    onClose()
  }, [hasChanges, onClose])

  // Handle escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        handleClose()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [handleClose])

  const hasErrors = errors.length > 0

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl shadow-2xl w-full max-w-3xl max-h-[90vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-white/10 flex-shrink-0">
          <div className="flex items-center gap-3">
            <Braces className="w-5 h-5 text-primary-400" />
            <h2 className="text-lg font-semibold text-white">{title}</h2>
          </div>

          <div className="flex items-center gap-3">
            {/* Mode toggle */}
            <div className="flex bg-black/40 rounded-md p-0.5 border border-white/10">
              <button
                type="button"
                onClick={mode === 'json' ? switchToVisual : undefined}
                className={`flex items-center gap-1.5 px-3 py-1 text-sm rounded-sm transition-colors ${
                  mode === 'visual'
                    ? 'bg-primary-500/20 text-primary-400'
                    : 'text-gray-500 hover:text-gray-300'
                }`}
              >
                <Eye className="w-3.5 h-3.5" />
                Visual
              </button>
              <button
                type="button"
                onClick={mode === 'visual' ? switchToJson : undefined}
                className={`flex items-center gap-1.5 px-3 py-1 text-sm rounded-sm transition-colors ${
                  mode === 'json'
                    ? 'bg-primary-500/20 text-primary-400'
                    : 'text-gray-500 hover:text-gray-300'
                }`}
              >
                <Code2 className="w-3.5 h-3.5" />
                JSON
              </button>
            </div>

            {/* Close button */}
            <button
              type="button"
              onClick={handleClose}
              className="p-1 text-gray-400 hover:text-white hover:bg-white/10 rounded"
            >
              <X className="w-5 h-5" />
            </button>
          </div>
        </div>

        {/* Body - scrollable */}
        <div className="flex-1 overflow-auto p-4">
          {mode === 'visual' ? (
            <SchemaVisualBuilder
              schema={editableSchema}
              onChange={handleVisualChange}
              errors={errors}
            />
          ) : (
            <SchemaCodeEditor
              value={jsonText}
              onChange={handleJsonChange}
              errors={errors}
            />
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between p-4 border-t border-white/10 flex-shrink-0 bg-black/20">
          <div className="text-xs text-gray-500">
            {hasChanges && <span className="text-yellow-400">Unsaved changes</span>}
          </div>

          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={handleClose}
              className="px-4 py-2 text-sm text-gray-300 hover:text-white hover:bg-white/10 rounded-lg"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleSave}
              disabled={hasErrors}
              className={`px-4 py-2 text-sm rounded-lg ${
                hasErrors
                  ? 'bg-gray-600 text-gray-400 cursor-not-allowed'
                  : 'bg-primary-500 text-white hover:bg-primary-600'
              }`}
            >
              Save Schema
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body
  )
}
