/**
 * Starlark/Python Editor for RaisinDB Functions
 *
 * A Monaco Editor configured for Starlark/Python with RaisinDB API completions.
 * Provides autocomplete for raisin.nodes, raisin.sql, raisin.http, etc.
 * Uses snake_case naming convention for Python/Starlark style.
 */

import { useCallback } from 'react'
import type * as Monaco from 'monaco-editor'
import { CodeEditor, type CodeEditorProps } from './CodeEditor'
import { registerStarlarkLanguage, STARLARK_LANGUAGE_ID } from '../languages/starlark'

export interface StarlarkEditorProps extends Omit<CodeEditorProps, 'language'> {
  // No additional props needed for now
}

/**
 * Starlark/Python Editor with RaisinDB API completions
 *
 * Extends CodeEditor with Starlark syntax highlighting and
 * RaisinDB-specific autocomplete suggestions (snake_case style).
 *
 * @example
 * ```tsx
 * <StarlarkEditor
 *   value={code}
 *   onChange={setCode}
 *   onSave={handleSave}
 *   onRun={handleRun}
 * />
 * ```
 */
export function StarlarkEditor({
  onBeforeMount,
  ...props
}: StarlarkEditorProps) {
  const handleBeforeMount = useCallback(
    (monaco: typeof Monaco) => {
      // Register Starlark language and completions
      registerStarlarkLanguage(monaco)
      onBeforeMount?.(monaco)
    },
    [onBeforeMount]
  )

  return (
    <CodeEditor
      {...props}
      language={STARLARK_LANGUAGE_ID}
      onBeforeMount={handleBeforeMount}
    />
  )
}

export default StarlarkEditor
