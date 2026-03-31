/**
 * Starlark/Python language support for RaisinDB
 *
 * Registers the Starlark language with Monaco Editor including:
 * - Syntax highlighting via Monarch tokenizer
 * - Language configuration (comments, brackets, indentation)
 * - RaisinDB API completions (snake_case style)
 */

import type { Monaco } from '@monaco-editor/react'
import { STARLARK_LANGUAGE_ID, starlarkLanguageConfig } from './config'
import { starlarkTokenizer } from './tokenizer'
import { registerRaisinStarlarkCompletionProvider } from './completions'

export { STARLARK_LANGUAGE_ID } from './config'
export { getRaisinStarlarkCompletions, registerRaisinStarlarkCompletionProvider } from './completions'
export type { RaisinApiCompletion } from './completions'

let starlarkLanguageRegistered = false

/**
 * Register Starlark language with Monaco Editor
 *
 * This function should be called in the editor's beforeMount callback.
 * It registers the language, tokenizer, configuration, and completion provider.
 *
 * @example
 * ```tsx
 * const handleBeforeMount = (monaco: Monaco) => {
 *   registerStarlarkLanguage(monaco)
 * }
 * ```
 */
export function registerStarlarkLanguage(monaco: Monaco): void {
  if (starlarkLanguageRegistered) {
    return
  }

  // Register the language
  monaco.languages.register({
    id: STARLARK_LANGUAGE_ID,
    extensions: ['.py', '.star', '.bzl'],
    aliases: ['Starlark', 'Python (Starlark)', 'starlark', 'python'],
    mimetypes: ['text/x-python', 'text/x-starlark'],
  })

  // Set language configuration
  monaco.languages.setLanguageConfiguration(STARLARK_LANGUAGE_ID, starlarkLanguageConfig)

  // Set Monarch tokenizer for syntax highlighting
  monaco.languages.setMonarchTokensProvider(STARLARK_LANGUAGE_ID, starlarkTokenizer)

  // Register RaisinDB API completion provider
  registerRaisinStarlarkCompletionProvider(monaco)

  starlarkLanguageRegistered = true
}

/**
 * Check if Starlark language is already registered
 */
export function isStarlarkLanguageRegistered(): boolean {
  return starlarkLanguageRegistered
}
