/**
 * Base Code Editor Component
 *
 * A Monaco Editor wrapper that provides a foundation for language-specific editors.
 * Includes theme support, keyboard shortcuts, and common editor options.
 */

import { useRef, useCallback } from 'react'
import Editor, { type OnMount, type BeforeMount, type OnChange } from '@monaco-editor/react'
import type { editor } from 'monaco-editor'
import type * as Monaco from 'monaco-editor'
import { registerRaisinDarkTheme, RAISIN_DARK_THEME } from '../themes/raisin-dark'

export interface CodeEditorProps {
  /** Current code value */
  value: string
  /** Called when the value changes */
  onChange?: (value: string) => void
  /** Editor language (javascript, typescript, python, sql, etc.) */
  language: string
  /** Editor height (default: 100%) */
  height?: string | number
  /** Whether the editor is read-only */
  readOnly?: boolean
  /** Additional editor options */
  options?: editor.IStandaloneEditorConstructionOptions
  /** Called when the editor is mounted */
  onMount?: (editor: editor.IStandaloneCodeEditor, monaco: typeof Monaco) => void
  /** Called before the editor mounts (use to register languages) */
  onBeforeMount?: (monaco: typeof Monaco) => void
  /** CSS class name */
  className?: string
  /** Theme name (default: raisin-dark) */
  theme?: string
  /** Keyboard shortcut handlers - receives current editor value to avoid stale state */
  onSave?: (value: string) => void
  onRun?: () => void
}

/**
 * Base Code Editor with Monaco
 *
 * Provides a generic code editor that can be extended for specific languages.
 * Includes keyboard shortcuts (Ctrl+S to save, Ctrl+Enter to run).
 *
 * @example
 * ```tsx
 * <CodeEditor
 *   value={code}
 *   onChange={setCode}
 *   language="javascript"
 *   onSave={handleSave}
 *   onRun={handleRun}
 * />
 * ```
 */
export function CodeEditor({
  value,
  onChange,
  language,
  height = '100%',
  readOnly = false,
  options,
  onMount: onMountProp,
  onBeforeMount: onBeforeMountProp,
  className,
  theme = RAISIN_DARK_THEME,
  onSave,
  onRun,
}: CodeEditorProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null)

  // Register theme and custom providers before editor mounts
  const handleBeforeMount: BeforeMount = useCallback(
    (monaco) => {
      registerRaisinDarkTheme(monaco)
      onBeforeMountProp?.(monaco)
    },
    [onBeforeMountProp]
  )

  // Store editor reference, add keyboard shortcuts, and call user's onMount
  const handleMount: OnMount = useCallback(
    (editor, monaco) => {
      editorRef.current = editor

      // Add keyboard shortcut for save (Ctrl+S / Cmd+S)
      // Pass current editor value directly to avoid stale React state
      if (onSave) {
        editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
          onSave(editor.getValue())
        })
      }

      // Add keyboard shortcut for run (Ctrl+Enter / Cmd+Enter)
      if (onRun) {
        editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
          onRun()
        })
      }

      onMountProp?.(editor, monaco)
    },
    [onMountProp, onSave, onRun]
  )

  // Handle value changes
  const handleChange: OnChange = useCallback(
    (newValue) => {
      onChange?.(newValue || '')
    },
    [onChange]
  )

  return (
    <div className={className} style={{ height, width: '100%' }}>
      <Editor
        height="100%"
        language={language}
        value={value}
        onChange={handleChange}
        onMount={handleMount}
        beforeMount={handleBeforeMount}
        theme={theme}
        options={{
          minimap: { enabled: false },
          fontSize: 14,
          lineNumbers: 'on',
          roundedSelection: true,
          scrollBeyondLastLine: false,
          automaticLayout: true,
          tabSize: 2,
          wordWrap: 'on',
          padding: { top: 16, bottom: 16 },
          readOnly,
          // Enable suggestions
          quickSuggestions: {
            other: true,
            comments: false,
            strings: true,
          },
          suggestOnTriggerCharacters: true,
          acceptSuggestionOnEnter: 'smart',
          // Hover settings
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
          bracketPairColorization: {
            enabled: true,
          },
          // Folding
          folding: true,
          foldingStrategy: 'auto',
          // Override with user options
          ...options,
        }}
      />
    </div>
  )
}

export default CodeEditor
