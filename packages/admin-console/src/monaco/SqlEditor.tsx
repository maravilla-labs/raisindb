/**
 * RaisinDB SQL Editor Component
 *
 * A Monaco Editor wrapper configured for RaisinDB SQL with syntax highlighting,
 * autocomplete, hover documentation, and real-time validation.
 */

import { useRef, useCallback, useState, useEffect } from 'react'
import Editor, { type OnMount, type BeforeMount, type OnChange } from '@monaco-editor/react'
import type { editor } from 'monaco-editor'
import type * as Monaco from 'monaco-editor'
import { registerRaisinSqlLanguage, registerSemanticProviders, LANGUAGE_ID } from './register'
import { useWasmValidator } from './validation'
import { workspacesApi } from '../api/workspaces'
import { getSchemaCache } from './schema-cache'

export interface SqlEditorProps {
  /** Current SQL value */
  value: string
  /** Called when the value changes */
  onChange?: (value: string) => void
  /** Editor height (default: 100%) */
  height?: string | number
  /** Whether the editor is read-only */
  readOnly?: boolean
  /** Additional editor options */
  options?: editor.IStandaloneEditorConstructionOptions
  /** Called when the editor is mounted */
  onMount?: (editor: editor.IStandaloneCodeEditor) => void
  /** CSS class name */
  className?: string
  /** Whether to enable real-time validation (default: true) */
  enableValidation?: boolean
  /** Repository name for fetching workspace table catalog */
  repo?: string
}

/**
 * SQL Editor with RaisinDB-specific syntax highlighting, autocomplete, hover docs,
 * and real-time validation.
 *
 * This component uses Monaco Editor with a custom language configuration
 * that understands RaisinDB DDL keywords (CREATE NODETYPE, property types, etc.)
 * and provides intelligent autocomplete suggestions. Validation is performed
 * using a WASM module running in a Web Worker for non-blocking performance.
 *
 * @example
 * ```tsx
 * <SqlEditor
 *   value={sql}
 *   onChange={setSql}
 *   height="300px"
 * />
 * ```
 */
export function SqlEditor({
  value,
  onChange,
  height = '100%',
  readOnly = false,
  options,
  onMount: onMountProp,
  className,
  enableValidation = true,
  repo,
}: SqlEditorProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null)
  const [monacoInstance, setMonacoInstance] = useState<typeof Monaco | null>(null)
  const [semanticProvidersRegistered, setSemanticProvidersRegistered] = useState(false)

  // Initialize the WASM validator with completion support
  const {
    validate,
    isReady: validatorReady,
    setTableCatalog,
    getCompletions,
    getSignatures,
    getAllFunctions,
  } = useWasmValidator(editorRef, monacoInstance, { debounceMs: 300 })

  // Register semantic completion providers when WASM is ready
  useEffect(() => {
    if (!validatorReady || !monacoInstance || semanticProvidersRegistered) {
      return
    }

    // Register semantic providers for completions and signature help
    registerSemanticProviders(monacoInstance, getCompletions, getSignatures)
    setSemanticProvidersRegistered(true)

    // Initialize function signatures in schema cache
    getAllFunctions()
      .then((functions) => {
        const cache = getSchemaCache()
        cache.setFunctions(functions)
        console.log('[SqlEditor] Schema cache initialized with', functions.length, 'functions')
      })
      .catch((error) => {
        console.error('[SqlEditor] Failed to get functions:', error)
      })
  }, [validatorReady, monacoInstance, semanticProvidersRegistered, getCompletions, getSignatures, getAllFunctions])

  // Fetch workspaces and set them as table catalog when repo changes
  // Note: We intentionally exclude setTableCatalog and validate from deps because we only
  // want this effect to run when repo or validatorReady changes. The callbacks are accessed
  // via closure and will always have the latest values when called.
  useEffect(() => {
    if (!repo || !validatorReady) {
      return
    }

    let cancelled = false

    const fetchWorkspaces = async () => {
      try {
        const workspaces = await workspacesApi.list(repo)
        if (cancelled) return

        // Convert workspace names to table catalog format
        const tableCatalog = workspaces.map((ws) => ({
          name: ws.name,
          columns: [], // Workspaces don't have column info, but we can add them later
        }))
        console.log('[SqlEditor] Setting table catalog for repo:', repo, 'tables:', tableCatalog)

        // Update schema cache with workspace names
        const cache = getSchemaCache()
        cache.updateWorkspaces(workspaces.map((ws) => ws.name))

        // Set catalog and re-validate when confirmed
        setTableCatalog(tableCatalog, () => {
          if (cancelled) return
          // Re-validate current content after catalog is confirmed set
          // This ensures workspace tables are recognized
          if (editorRef.current) {
            const currentValue = editorRef.current.getValue()
            if (currentValue) {
              console.log('[SqlEditor] Catalog set, re-validating content')
              validate(currentValue)
            }
          }
        })
      } catch (error) {
        console.error('Failed to fetch workspaces for table catalog:', error)
      }
    }

    fetchWorkspaces()

    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo, validatorReady])

  // Register the RaisinSQL language before the editor mounts
  const handleBeforeMount: BeforeMount = useCallback((monaco) => {
    registerRaisinSqlLanguage(monaco)
  }, [])

  // Store editor reference and call user's onMount
  const handleMount: OnMount = useCallback(
    (editor, monaco) => {
      editorRef.current = editor
      setMonacoInstance(monaco)
      onMountProp?.(editor)
    },
    [onMountProp]
  )

  // Handle value changes
  const handleChange: OnChange = useCallback(
    (newValue) => {
      onChange?.(newValue || '')

      // Validate the new value
      if (enableValidation && newValue) {
        validate(newValue)
      }
    },
    [onChange, validate, enableValidation]
  )

  // Validate initial value when validator becomes ready
  useEffect(() => {
    if (enableValidation && validatorReady && value) {
      validate(value)
    }
  }, [validatorReady, enableValidation]) // Only run when validator becomes ready

  return (
    <div className={className} style={{ height, width: '100%' }}>
      <Editor
        height="100%"
        language={LANGUAGE_ID}
        value={value}
        onChange={handleChange}
        onMount={handleMount}
        beforeMount={handleBeforeMount}
        theme="raisin-dark"
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

export default SqlEditor
