/**
 * RaisinDB SQL language registration for Monaco Editor
 *
 * This module registers the custom RaisinSQL language with Monaco,
 * including syntax highlighting, autocomplete, and hover documentation.
 */

import type { Monaco } from '@monaco-editor/react'
import type { IDisposable } from 'monaco-editor'
import { LANGUAGE_ID, languageConfig } from './language-config'
import { monarchTokenizer } from './tokenizer'
import { registerCompletionProvider } from './completion'
import { registerHoverProvider } from './hover'
import type { CompletionResult } from './validation/types'
import type { FunctionSignature } from './schema-cache'

let isRegistered = false
let semanticProvidersDisposables: IDisposable[] = []

/**
 * Register the RaisinSQL language with Monaco Editor
 *
 * This should be called once when the application starts or
 * before the first editor instance is created.
 *
 * @param monaco - The Monaco editor instance
 */
export function registerRaisinSqlLanguage(monaco: Monaco): void {
  if (isRegistered) {
    return
  }

  // Register the language
  monaco.languages.register({
    id: LANGUAGE_ID,
    extensions: ['.raisinsql', '.rsql'],
    aliases: ['RaisinSQL', 'raisinsql', 'rsql'],
    mimetypes: ['application/x-raisinsql'],
  })

  // Set language configuration (brackets, comments, etc.)
  monaco.languages.setLanguageConfiguration(LANGUAGE_ID, languageConfig)

  // Set Monarch tokenizer for syntax highlighting
  monaco.languages.setMonarchTokensProvider(LANGUAGE_ID, monarchTokenizer)

  // Register completion provider for autocomplete
  registerCompletionProvider(monaco)

  // Register hover provider for documentation
  registerHoverProvider(monaco)

  // Define custom theme rules for RaisinSQL-specific tokens
  monaco.editor.defineTheme('raisin-dark', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      // DDL Statement keywords (CREATE, ALTER, DROP) - bold orange
      { token: 'keyword.statement.raisinsql', foreground: 'ff9f43', fontStyle: 'bold' },
      // Schema objects (NODETYPE, ARCHETYPE, ELEMENTTYPE) - cyan
      { token: 'keyword.schemaObject.raisinsql', foreground: '00d9ff', fontStyle: 'bold' },
      // Property types (String, Number, etc.) - green
      { token: 'type.propertyType.raisinsql', foreground: '7bed9f' },
      // Modifiers (REQUIRED, FULLTEXT, etc.) - purple
      { token: 'keyword.modifier.raisinsql', foreground: 'a29bfe' },
      // Flags (CASCADE, ORDERABLE, etc.) - yellow
      { token: 'keyword.flag.raisinsql', foreground: 'ffeaa7' },
      // Clauses (EXTENDS, PROPERTIES, etc.) - light blue
      { token: 'keyword.clause.raisinsql', foreground: '74b9ff' },
      // Functions - bright yellow
      { token: 'function.raisinsql', foreground: 'fdcb6e' },
      // Standard SQL keywords - blue
      { token: 'keyword.raisinsql', foreground: '569cd6' },
      // Type identifiers (namespace:type) - teal
      { token: 'type.identifier.raisinsql', foreground: '26de81' },
      // Strings - orange
      { token: 'string.raisinsql', foreground: 'ce9178' },
      { token: 'string.quote.raisinsql', foreground: 'ce9178' },
      { token: 'string.escape.raisinsql', foreground: 'd7ba7d' },
      // Numbers - light green
      { token: 'number.raisinsql', foreground: 'b5cea8' },
      // Operators
      { token: 'operator.raisinsql', foreground: 'd4d4d4' },
      { token: 'operator.cast.raisinsql', foreground: '569cd6' },
      // Comments - green
      { token: 'comment.raisinsql', foreground: '6a9955' },
      // Identifiers
      { token: 'identifier.raisinsql', foreground: '9cdcfe' },

      // Cypher-specific tokens (embedded in CYPHER() function calls)
      // Cypher keywords - purple (distinct from SQL blue)
      { token: 'keyword.cypher.raisinsql', foreground: 'c586c0', fontStyle: 'bold' },
      // Cypher functions - yellow
      { token: 'function.cypher.raisinsql', foreground: 'dcdcaa' },
      // Labels and relationship types (:Person, :KNOWS) - teal
      { token: 'type.label.cypher.raisinsql', foreground: '4ec9b0' },
      // Variables - light blue
      { token: 'variable.cypher.raisinsql', foreground: '9cdcfe' },
      // Property access (.name, .age) - light blue
      { token: 'variable.property.cypher.raisinsql', foreground: '9cdcfe' },
      // Arrow operators (->, <-) - gray
      { token: 'operator.arrow.cypher.raisinsql', foreground: 'd4d4d4' },
      // Other operators - gray
      { token: 'operator.cypher.raisinsql', foreground: 'd4d4d4' },
      // Inner strings in Cypher (double-quoted in single-quoted context) - orange
      { token: 'string.inner.cypher.raisinsql', foreground: 'ce9178' },
      // Default Cypher string content - muted orange
      { token: 'string.cypher.raisinsql', foreground: 'd7ba7d' },
      // Escape sequences in Cypher strings
      { token: 'string.escape.cypher.raisinsql', foreground: 'd7ba7d' },
      // Numbers in Cypher - light green
      { token: 'number.cypher.raisinsql', foreground: 'b5cea8' },
      // Delimiters in Cypher
      { token: 'delimiter.parenthesis.cypher.raisinsql', foreground: 'ffd700' },
      { token: 'delimiter.bracket.cypher.raisinsql', foreground: 'da70d6' },
      { token: 'delimiter.curly.cypher.raisinsql', foreground: '179fff' },

      // Reference string tokens for REFERENCES('workspace:/path')
      // Workspace name - teal/cyan (like identifiers)
      { token: 'variable.reference.workspace.raisinsql', foreground: '4ec9b0', fontStyle: 'bold' },
      // Colon separator - gray
      { token: 'operator.reference.separator.raisinsql', foreground: 'd4d4d4' },
      // Path - orange (like strings but slightly different)
      { token: 'string.reference.path.raisinsql', foreground: 'e9a66c' },
    ],
    colors: {
      'editor.background': '#1e1e1e',
      'editor.foreground': '#d4d4d4',
    },
  })

  isRegistered = true
}

/**
 * Get the language ID for RaisinSQL
 */
export function getRaisinSqlLanguageId(): string {
  return LANGUAGE_ID
}

/**
 * Check if RaisinSQL language is already registered
 */
export function isRaisinSqlRegistered(): boolean {
  return isRegistered
}

/**
 * Register semantic completion and signature help providers
 *
 * Call this after the WASM validator is ready to enable context-aware
 * completions (tables, columns, functions) and parameter hints.
 *
 * @param monaco - The Monaco editor instance
 * @param getCompletions - Function to get completions from WASM worker
 * @param getSignatures - Function to get function signatures from WASM worker
 */
export function registerSemanticProviders(
  monaco: Monaco,
  getCompletions: (sql: string, offset: number) => Promise<CompletionResult | null>,
  getSignatures: (functionName: string) => Promise<FunctionSignature[] | null>
): void {
  // Dispose previous semantic providers if any
  for (const disposable of semanticProvidersDisposables) {
    disposable.dispose()
  }
  semanticProvidersDisposables = []

  // Register semantic completion provider
  const completionDisposable = monaco.languages.registerCompletionItemProvider(
    LANGUAGE_ID,
    {
      triggerCharacters: [' ', '.', '(', ',', "'"],
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      async provideCompletionItems(model: any, position: any, _context: any, _token: any) {
        const word = model.getWordUntilPosition(position)
        const range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn,
        }

        try {
          const sql = model.getValue()
          const offset = model.getOffsetAt(position)
          const result = await getCompletions(sql, offset)

          if (result && result.items.length > 0) {
            return {
              suggestions: result.items.map((item) => ({
                label: item.label,
                kind: wasmKindToMonacoKind(item.kind, monaco.languages.CompletionItemKind),
                insertText: item.insert_text,
                insertTextRules:
                  item.insert_text_format === 'snippet'
                    ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
                    : undefined,
                detail: item.detail,
                documentation: item.documentation ? { value: item.documentation } : undefined,
                sortText: item.sort_text ?? item.label,
                filterText: item.filter_text ?? item.label,
                range,
              })),
            }
          }
        } catch (error) {
          console.error('[SemanticCompletion] Error:', error)
        }

        // Return empty - fallback provider will handle it
        return { suggestions: [] }
      },
    }
  )
  semanticProvidersDisposables.push(completionDisposable)

  // Register signature help provider
  const signatureDisposable = monaco.languages.registerSignatureHelpProvider(LANGUAGE_ID, {
    signatureHelpTriggerCharacters: ['(', ','],
    signatureHelpRetriggerCharacters: [','],

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    async provideSignatureHelp(model: any, position: any, _token: any, _context: any) {
      const funcContext = findFunctionContext(model, position)

      if (!funcContext) {
        return null
      }

      try {
        const signatures = await getSignatures(funcContext.functionName)

        if (!signatures || signatures.length === 0) {
          return null
        }

        const monacoSignatures = signatures.map((sig) => ({
          label: `${sig.name}(${sig.params.join(', ')}) -> ${sig.returnType}`,
          documentation: {
            value: `**Category:** ${sig.category}`,
          },
          parameters: sig.params.map((param) => ({
            label: param,
            documentation: `Parameter: ${param}`,
          })),
        }))

        return {
          value: {
            signatures: monacoSignatures,
            activeSignature: 0,
            activeParameter: Math.min(funcContext.argIndex, signatures[0].params.length - 1),
          },
          dispose: () => {},
        }
      } catch (error) {
        console.error('[SignatureHelp] Error:', error)
        return null
      }
    },
  })
  semanticProvidersDisposables.push(signatureDisposable)

  console.log('[RaisinSQL] Semantic providers registered')
}

/**
 * Map WASM completion kind to Monaco completion kind
 */
function wasmKindToMonacoKind(
  kind: string,
  kinds: typeof import('monaco-editor').languages.CompletionItemKind
): number {
  switch (kind) {
    case 'keyword':
      return kinds.Keyword
    case 'table':
      return kinds.Class
    case 'column':
      return kinds.Field
    case 'function':
      return kinds.Function
    case 'aggregate':
      return kinds.Function
    case 'snippet':
      return kinds.Snippet
    case 'type':
      return kinds.TypeParameter
    case 'alias':
      return kinds.Variable
    case 'operator':
      return kinds.Operator
    default:
      return kinds.Text
  }
}

/**
 * Find function context at cursor position for signature help
 */
function findFunctionContext(
  model: import('monaco-editor').editor.ITextModel,
  position: import('monaco-editor').Position
): { functionName: string; argIndex: number } | null {
  const textBefore = model.getValueInRange({
    startLineNumber: Math.max(1, position.lineNumber - 10),
    startColumn: 1,
    endLineNumber: position.lineNumber,
    endColumn: position.column,
  })

  // Track parentheses depth and commas
  let parenDepth = 0
  let argIndex = 0
  let functionStart = -1

  // Walk backwards through the text
  for (let i = textBefore.length - 1; i >= 0; i--) {
    const char = textBefore[i]

    if (char === ')') {
      parenDepth++
    } else if (char === '(') {
      if (parenDepth === 0) {
        functionStart = i
        break
      }
      parenDepth--
    } else if (char === ',' && parenDepth === 0) {
      argIndex++
    }
  }

  if (functionStart < 0) {
    return null
  }

  // Extract function name
  let funcEnd = functionStart - 1
  while (funcEnd >= 0 && /\s/.test(textBefore[funcEnd])) {
    funcEnd--
  }

  if (funcEnd < 0) {
    return null
  }

  let funcStart = funcEnd
  while (funcStart > 0 && /[a-zA-Z0-9_]/.test(textBefore[funcStart - 1])) {
    funcStart--
  }

  const functionName = textBefore.slice(funcStart, funcEnd + 1)

  if (!functionName || /^(SELECT|FROM|WHERE|AND|OR|IN|EXISTS|VALUES)$/i.test(functionName)) {
    return null
  }

  return { functionName: functionName.toUpperCase(), argIndex }
}

// Re-export for convenience
export { LANGUAGE_ID } from './language-config'
