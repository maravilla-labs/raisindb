/**
 * Function Signature Help Provider
 *
 * Shows function parameter hints as the user types inside function parentheses.
 */

import type { languages, editor, Position, CancellationToken } from 'monaco-editor'
import { LANGUAGE_ID } from './language-config'
import type { FunctionSignature } from './schema-cache'

// =============================================================================
// Types
// =============================================================================

type SignatureGetter = (functionName: string) => Promise<FunctionSignature[] | null>

// =============================================================================
// Helpers
// =============================================================================

/**
 * Find function context at cursor position
 *
 * @returns Function name and argument index, or null if not in a function call
 */
function findFunctionContext(
  model: editor.ITextModel,
  position: Position
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
        // Found our opening paren
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

  // Extract function name (identifier before the opening paren)
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

// =============================================================================
// Provider
// =============================================================================

/**
 * Create a signature help provider for SQL functions
 *
 * @param _monaco - Monaco editor module (unused but kept for consistency)
 * @param getSignatures - Function to get signatures from WASM worker
 */
export function createSignatureHelpProvider(
  _monaco: typeof import('monaco-editor'),
  getSignatures: SignatureGetter
): languages.SignatureHelpProvider {
  return {
    signatureHelpTriggerCharacters: ['(', ','],
    signatureHelpRetriggerCharacters: [','],

    async provideSignatureHelp(
      model: editor.ITextModel,
      position: Position,
      _token: CancellationToken,
      _context: languages.SignatureHelpContext
    ): Promise<languages.SignatureHelpResult | null> {
      const funcContext = findFunctionContext(model, position)

      if (!funcContext) {
        return null
      }

      try {
        const signatures = await getSignatures(funcContext.functionName)

        if (!signatures || signatures.length === 0) {
          return null
        }

        const monacoSignatures: languages.SignatureInformation[] = signatures.map((sig) => ({
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
        console.error('[SignatureHelp] Error getting signatures:', error)
        return null
      }
    },
  }
}

/**
 * Register the signature help provider
 */
export function registerSignatureHelpProvider(
  monaco: typeof import('monaco-editor'),
  getSignatures: SignatureGetter
): void {
  monaco.languages.registerSignatureHelpProvider(
    LANGUAGE_ID,
    createSignatureHelpProvider(monaco, getSignatures)
  )
}
