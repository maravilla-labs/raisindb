/**
 * JavaScript Editor for RaisinDB Functions
 *
 * A Monaco Editor configured for JavaScript with RaisinDB API completions.
 * Provides autocomplete for raisin.nodes, raisin.sql, raisin.http, etc.
 */

import { useCallback } from 'react'
import type * as Monaco from 'monaco-editor'
import { CodeEditor, type CodeEditorProps } from './CodeEditor'
import { registerRaisinJsCompletionProvider } from '../languages/javascript/completions'

export interface JavaScriptEditorProps extends Omit<CodeEditorProps, 'language'> {
  /** Override language to typescript if needed */
  language?: 'javascript' | 'typescript'
}

let raisinCompletionsRegistered = false

/**
 * JavaScript Editor with RaisinDB API completions
 *
 * Extends CodeEditor with RaisinDB-specific autocomplete suggestions
 * for the raisin.* global object available in serverless functions.
 *
 * @example
 * ```tsx
 * <JavaScriptEditor
 *   value={code}
 *   onChange={setCode}
 *   onSave={handleSave}
 *   onRun={handleRun}
 * />
 * ```
 */
export function JavaScriptEditor({
  language = 'javascript',
  onBeforeMount,
  ...props
}: JavaScriptEditorProps) {
  const handleBeforeMount = useCallback(
    (monaco: typeof Monaco) => {
      // Register RaisinDB completions only once
      if (!raisinCompletionsRegistered) {
        registerRaisinJsCompletionProvider(monaco)
        raisinCompletionsRegistered = true
      }

      onBeforeMount?.(monaco)
    },
    [onBeforeMount]
  )

  return (
    <CodeEditor
      {...props}
      language={language}
      onBeforeMount={handleBeforeMount}
    />
  )
}

export default JavaScriptEditor
