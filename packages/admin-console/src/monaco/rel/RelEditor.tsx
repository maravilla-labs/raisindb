/**
 * REL (Raisin Expression Language) Editor Component
 *
 * A compact Monaco Editor wrapper for editing REL expressions with
 * syntax highlighting, autocomplete, hover documentation, and validation.
 */

import { useRef, useCallback, useState, useEffect } from 'react'
import Editor, { type OnMount, type BeforeMount, type OnChange } from '@monaco-editor/react'
import type { editor } from 'monaco-editor'
import type * as Monaco from 'monaco-editor'
import { registerRelLanguage, REL_LANGUAGE_ID, updateRelCompletionOptions } from './register'

export interface RelEditorProps {
  /** Current REL expression value */
  value: string
  /** Called when the value changes */
  onChange?: (value: string) => void
  /** Editor height (default: '100px') */
  height?: string | number
  /** Placeholder text shown when editor is empty */
  placeholder?: string
  /** Whether the editor is disabled/read-only */
  disabled?: boolean
  /** Whether to show line numbers (default: false) */
  showLineNumbers?: boolean
  /** Available field names for completion suggestions */
  availableFields?: string[]
  /** Available relation types for VIA completion (from raisin:access_control/relation-types/) */
  relationTypes?: string[]
  /** Additional CSS class name */
  className?: string
  /** Validation errors to display */
  errors?: Array<{
    line: number
    column: number
    endLine: number
    endColumn: number
    message: string
  }>
}

/**
 * Compact Monaco Editor for REL expressions.
 *
 * Features:
 * - Syntax highlighting for REL language
 * - Autocomplete for functions, keywords, and field access
 * - Hover documentation
 * - Function signature help
 * - Error markers for validation
 *
 * @example
 * ```tsx
 * <RelEditor
 *   value={condition}
 *   onChange={setCondition}
 *   placeholder="input.value > 10 && input.status == 'active'"
 * />
 * ```
 */
export function RelEditor({
  value,
  onChange,
  height = '100px',
  placeholder,
  disabled = false,
  showLineNumbers = false,
  availableFields,
  relationTypes,
  className,
  errors,
}: RelEditorProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null)
  const [monacoInstance, setMonacoInstance] = useState<typeof Monaco | null>(null)

  // Register the REL language before the editor mounts
  const handleBeforeMount: BeforeMount = useCallback((monaco) => {
    registerRelLanguage(monaco, availableFields)
  }, [availableFields])

  // Update completion options when availableFields or relationTypes change
  useEffect(() => {
    updateRelCompletionOptions({ availableFields, relationTypes })
  }, [availableFields, relationTypes])

  // Store editor reference
  const handleMount: OnMount = useCallback((editor, monaco) => {
    editorRef.current = editor
    setMonacoInstance(monaco)

    // If there's a placeholder and value is empty, show it
    if (placeholder && !value) {
      // We'll handle placeholder via decoration
      updatePlaceholder(editor, monaco, value, placeholder)
    }
  }, [placeholder, value])

  // Handle value changes
  const handleChange: OnChange = useCallback((newValue) => {
    onChange?.(newValue || '')

    // Update placeholder visibility
    if (editorRef.current && monacoInstance && placeholder) {
      updatePlaceholder(editorRef.current, monacoInstance, newValue || '', placeholder)
    }
  }, [onChange, monacoInstance, placeholder])

  // Update error markers when errors change
  useEffect(() => {
    if (!editorRef.current || !monacoInstance) return

    const model = editorRef.current.getModel()
    if (!model) return

    const markers: Monaco.editor.IMarkerData[] = (errors || []).map((err) => ({
      severity: monacoInstance.MarkerSeverity.Error,
      startLineNumber: err.line,
      startColumn: err.column,
      endLineNumber: err.endLine,
      endColumn: err.endColumn,
      message: err.message,
    }))

    monacoInstance.editor.setModelMarkers(model, 'rel', markers)
  }, [errors, monacoInstance])

  // Stop all keyboard events from propagating to parent handlers
  // This prevents flow-designer delete/backspace handlers from triggering
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    e.stopPropagation()
  }, [])

  return (
    <div
      className={className}
      style={{ height, width: '100%' }}
      onKeyDown={handleKeyDown}
      onKeyUp={handleKeyDown}
      onKeyPress={handleKeyDown}
    >
      <Editor
        height="100%"
        language={REL_LANGUAGE_ID}
        value={value}
        onChange={handleChange}
        onMount={handleMount}
        beforeMount={handleBeforeMount}
        theme="rel-dark"
        options={{
          // Compact appearance
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          lineNumbers: showLineNumbers ? 'on' : 'off',
          glyphMargin: false,
          folding: false,
          wordWrap: 'on',
          wrappingStrategy: 'advanced',
          padding: { top: 8, bottom: 8 },
          scrollbar: {
            vertical: 'hidden',
            horizontal: 'auto',
            verticalScrollbarSize: 0,
            horizontalScrollbarSize: 6,
          },
          overviewRulerLanes: 0,
          hideCursorInOverviewRuler: true,
          overviewRulerBorder: false,

          // Typography
          fontSize: 13,
          fontFamily: "'JetBrains Mono', 'Fira Code', 'Consolas', monospace",
          lineHeight: 20,

          // Editing behavior
          renderLineHighlight: 'none',
          renderWhitespace: 'none',
          autoIndent: 'none',
          tabSize: 2,

          // Suggestions
          quickSuggestions: true,
          suggestOnTriggerCharacters: true,
          acceptSuggestionOnEnter: 'smart',

          // Hover
          hover: {
            enabled: true,
            delay: 300,
          },

          // Parameter hints
          parameterHints: {
            enabled: true,
          },

          // Bracket matching
          matchBrackets: 'always',

          // Read-only when disabled
          readOnly: disabled,

          // Accessibility
          accessibilitySupport: 'auto',
          ariaLabel: 'REL Expression Editor',
        }}
      />
    </div>
  )
}

// Placeholder decoration collection ID
let placeholderDecorationIds: string[] = []

function updatePlaceholder(
  editor: editor.IStandaloneCodeEditor,
  monaco: typeof Monaco,
  value: string,
  placeholder: string
): void {
  const model = editor.getModel()
  if (!model) return

  // Remove existing placeholder decorations
  placeholderDecorationIds = editor.deltaDecorations(placeholderDecorationIds, [])

  // Show placeholder only when empty
  if (!value || value.trim() === '') {
    placeholderDecorationIds = editor.deltaDecorations([], [{
      range: new monaco.Range(1, 1, 1, 1),
      options: {
        after: {
          content: placeholder,
          inlineClassName: 'rel-editor-placeholder',
        },
      },
    }])
  }
}

export default RelEditor
