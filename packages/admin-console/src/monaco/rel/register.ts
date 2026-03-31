/**
 * REL (Raisin Expression Language) registration for Monaco Editor
 *
 * This module registers the custom REL language with Monaco,
 * including syntax highlighting, autocomplete, hover documentation,
 * and signature help.
 */

import type { Monaco } from '@monaco-editor/react'
import { REL_LANGUAGE_ID, relLanguageConfig } from './language-config'
import { relMonarchTokenizer } from './tokenizer'
import { registerRelCompletionProvider } from './completion'
import { registerRelHoverProvider } from './hover'
import { registerRelSignatureHelpProvider } from './signature-help'

let isRegistered = false

/**
 * Register the REL language with Monaco Editor
 *
 * This should be called once when the application starts or
 * before the first editor instance is created.
 *
 * @param monaco - The Monaco editor instance
 * @param availableFields - Optional list of available field names for completion
 */
export function registerRelLanguage(monaco: Monaco, availableFields?: string[]): void {
  if (isRegistered) {
    return
  }

  // Register the language
  monaco.languages.register({
    id: REL_LANGUAGE_ID,
    extensions: ['.rel'],
    aliases: ['REL', 'rel', 'Raisin Expression Language'],
    mimetypes: ['text/x-rel'],
  })

  // Set language configuration (brackets, auto-closing pairs)
  monaco.languages.setLanguageConfiguration(REL_LANGUAGE_ID, relLanguageConfig)

  // Set Monarch tokenizer for syntax highlighting
  monaco.languages.setMonarchTokensProvider(REL_LANGUAGE_ID, relMonarchTokenizer)

  // Register completion provider for autocomplete
  registerRelCompletionProvider(monaco, { availableFields })

  // Register hover provider for documentation
  registerRelHoverProvider(monaco)

  // Register signature help provider for function parameters
  registerRelSignatureHelpProvider(monaco)

  // Define custom theme rules for REL-specific tokens
  monaco.editor.defineTheme('rel-dark', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      // RELATES keywords
      { token: 'keyword.relates.rel', foreground: '00d9ff', fontStyle: 'bold' },
      { token: 'keyword.clause.rel', foreground: '74b9ff' },
      { token: 'keyword.modifier.rel', foreground: 'a29bfe' },
      // Field access (input.field, context.user)
      { token: 'variable.field.rel', foreground: '9cdcfe' },
      // Functions (contains, startsWith, endsWith)
      { token: 'function.rel', foreground: 'dcdcaa' },
      // Boolean keywords (true, false)
      { token: 'keyword.boolean.rel', foreground: '569cd6' },
      // Null keyword
      { token: 'keyword.null.rel', foreground: '569cd6' },
      // Logical combinators (&&, ||)
      { token: 'keyword.combinator.rel', foreground: 'c586c0' },
      // NOT operator
      { token: 'operator.not.rel', foreground: 'c586c0' },
      // Comparison operators (==, !=, >, <, >=, <=)
      { token: 'operator.comparison.rel', foreground: 'd4d4d4' },
      // Other operators
      { token: 'operator.rel', foreground: 'd4d4d4' },
      // Numbers
      { token: 'number.rel', foreground: 'b5cea8' },
      // Strings
      { token: 'string.rel', foreground: 'ce9178' },
      { token: 'string.quote.rel', foreground: 'ce9178' },
      { token: 'string.escape.rel', foreground: 'd7ba7d' },
      // Identifiers
      { token: 'identifier.rel', foreground: '9cdcfe' },
      // Delimiters
      { token: 'delimiter.rel', foreground: 'd4d4d4' },
    ],
    colors: {
      'editor.background': '#1e1e1e',
      'editor.foreground': '#d4d4d4',
    },
  })

  isRegistered = true
}

/**
 * Get the language ID for REL
 */
export function getRelLanguageId(): string {
  return REL_LANGUAGE_ID
}

/**
 * Check if REL language is already registered
 */
export function isRelRegistered(): boolean {
  return isRegistered
}

// Re-export for convenience
export { REL_LANGUAGE_ID } from './language-config'
export { updateRelCompletionOptions } from './completion'
export type { RelCompletionOptions } from './completion'
