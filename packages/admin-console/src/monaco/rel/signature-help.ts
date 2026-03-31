/**
 * REL (Raisin Expression Language) signature help provider for Monaco Editor
 */

import type { languages, editor, Position, CancellationToken } from 'monaco-editor'
import { REL_LANGUAGE_ID } from './language-config'

interface MethodSignature {
  name: string
  label: string
  documentation: string
  parameters: Array<{
    label: string
    documentation: string
  }>
}

// Method signatures for method-chaining syntax (e.g., input.name.contains('test'))
const methodSignatures: Map<string, MethodSignature> = new Map([
  // Universal methods
  ['length', {
    name: 'length',
    label: '.length() -> number',
    documentation: 'Get length of string, array, or number of object keys.',
    parameters: [],
  }],
  ['isempty', {
    name: 'isEmpty',
    label: '.isEmpty() -> boolean',
    documentation: 'Check if empty (empty string, empty array, empty object, or null).',
    parameters: [],
  }],
  ['isnotempty', {
    name: 'isNotEmpty',
    label: '.isNotEmpty() -> boolean',
    documentation: 'Check if not empty and not null.',
    parameters: [],
  }],

  // Polymorphic methods
  ['contains', {
    name: 'contains',
    label: '.contains(value) -> boolean',
    documentation: 'String: check if contains substring. Array: check if contains element.',
    parameters: [
      { label: 'value', documentation: 'The substring or element to search for.' },
    ],
  }],

  // String methods
  ['startswith', {
    name: 'startsWith',
    label: '.startsWith(prefix) -> boolean',
    documentation: 'Check if string starts with the given prefix.',
    parameters: [
      { label: 'prefix', documentation: 'The prefix to check for.' },
    ],
  }],
  ['endswith', {
    name: 'endsWith',
    label: '.endsWith(suffix) -> boolean',
    documentation: 'Check if string ends with the given suffix.',
    parameters: [
      { label: 'suffix', documentation: 'The suffix to check for.' },
    ],
  }],
  ['tolowercase', {
    name: 'toLowerCase',
    label: '.toLowerCase() -> string',
    documentation: 'Convert string to lowercase.',
    parameters: [],
  }],
  ['touppercase', {
    name: 'toUpperCase',
    label: '.toUpperCase() -> string',
    documentation: 'Convert string to uppercase.',
    parameters: [],
  }],
  ['trim', {
    name: 'trim',
    label: '.trim() -> string',
    documentation: 'Remove leading and trailing whitespace.',
    parameters: [],
  }],
  ['substring', {
    name: 'substring',
    label: '.substring(start, end?) -> string',
    documentation: 'Extract substring from start index to optional end index.',
    parameters: [
      { label: 'start', documentation: 'Start index (0-based).' },
      { label: 'end', documentation: 'Optional end index (exclusive). Defaults to end of string.' },
    ],
  }],

  // Array methods
  ['first', {
    name: 'first',
    label: '.first() -> any',
    documentation: 'Get first element of array (null if empty).',
    parameters: [],
  }],
  ['last', {
    name: 'last',
    label: '.last() -> any',
    documentation: 'Get last element of array (null if empty).',
    parameters: [],
  }],
  ['indexof', {
    name: 'indexOf',
    label: '.indexOf(element) -> number',
    documentation: 'Get index of element in array (-1 if not found).',
    parameters: [
      { label: 'element', documentation: 'The element to find.' },
    ],
  }],
  ['join', {
    name: 'join',
    label: '.join(separator?) -> string',
    documentation: 'Join array elements into a string with optional separator.',
    parameters: [
      { label: 'separator', documentation: 'Optional separator between elements. Defaults to empty string.' },
    ],
  }],

  // Path methods
  ['parent', {
    name: 'parent',
    label: '.parent(n?) -> string',
    documentation: 'Get parent path. Optional n for number of levels up.',
    parameters: [
      { label: 'n', documentation: 'Optional number of levels up. Defaults to 1.' },
    ],
  }],
  ['ancestor', {
    name: 'ancestor',
    label: '.ancestor(depth) -> string',
    documentation: 'Get ancestor at absolute depth from root.',
    parameters: [
      { label: 'depth', documentation: 'The absolute depth from root.' },
    ],
  }],
  ['ancestorof', {
    name: 'ancestorOf',
    label: '.ancestorOf(path) -> boolean',
    documentation: 'Check if this path is an ancestor of the given path.',
    parameters: [
      { label: 'path', documentation: 'The path to check against.' },
    ],
  }],
  ['descendantof', {
    name: 'descendantOf',
    label: '.descendantOf(path) -> boolean',
    documentation: 'Check if this path is a descendant of the given path.',
    parameters: [
      { label: 'path', documentation: 'The ancestor path to check against.' },
    ],
  }],
  ['childof', {
    name: 'childOf',
    label: '.childOf(path) -> boolean',
    documentation: 'Check if this path is a direct child of the given path.',
    parameters: [
      { label: 'path', documentation: 'The parent path to check against.' },
    ],
  }],
  ['depth', {
    name: 'depth',
    label: '.depth() -> number',
    documentation: 'Get hierarchy depth of path.',
    parameters: [],
  }],
])

// Find method context in method-chaining syntax (e.g., input.name.contains('test'))
function findMethodContext(
  model: editor.ITextModel,
  position: Position
): { methodName: string; argIndex: number } | null {
  const textBefore = model.getValueInRange({
    startLineNumber: 1,
    startColumn: 1,
    endLineNumber: position.lineNumber,
    endColumn: position.column,
  })

  // Track parentheses depth and commas
  let parenDepth = 0
  let argIndex = 0
  let openParenPos = -1

  // Walk backwards through the text to find the opening paren
  for (let i = textBefore.length - 1; i >= 0; i--) {
    const char = textBefore[i]

    if (char === ')') {
      parenDepth++
    } else if (char === '(') {
      if (parenDepth === 0) {
        openParenPos = i
        break
      }
      parenDepth--
    } else if (char === ',' && parenDepth === 0) {
      argIndex++
    }
  }

  if (openParenPos < 0) {
    return null
  }

  // Extract method name (identifier before the open paren)
  let methodEnd = openParenPos - 1
  while (methodEnd >= 0 && /\s/.test(textBefore[methodEnd])) {
    methodEnd--
  }

  if (methodEnd < 0) {
    return null
  }

  let methodStart = methodEnd
  while (methodStart > 0 && /[a-zA-Z0-9_]/.test(textBefore[methodStart - 1])) {
    methodStart--
  }

  const methodName = textBefore.slice(methodStart, methodEnd + 1)

  if (!methodName) {
    return null
  }

  // Check if there's a dot before the method name (confirming it's a method call)
  let dotPos = methodStart - 1
  while (dotPos >= 0 && /\s/.test(textBefore[dotPos])) {
    dotPos--
  }

  if (dotPos < 0 || textBefore[dotPos] !== '.') {
    return null // Not a method call
  }

  return { methodName: methodName.toLowerCase(), argIndex }
}

export function createRelSignatureHelpProvider(
  _monaco: typeof import('monaco-editor')
): languages.SignatureHelpProvider {
  return {
    signatureHelpTriggerCharacters: ['(', ','],
    signatureHelpRetriggerCharacters: [','],

    provideSignatureHelp(
      model: editor.ITextModel,
      position: Position,
      _token: CancellationToken,
      _context: languages.SignatureHelpContext
    ): languages.ProviderResult<languages.SignatureHelpResult> {
      const methodContext = findMethodContext(model, position)

      if (!methodContext) {
        return null
      }

      const signature = methodSignatures.get(methodContext.methodName)

      if (!signature) {
        return null
      }

      return {
        value: {
          signatures: [{
            label: signature.label,
            documentation: signature.documentation,
            parameters: signature.parameters,
          }],
          activeSignature: 0,
          activeParameter: Math.min(methodContext.argIndex, Math.max(signature.parameters.length - 1, 0)),
        },
        dispose: () => {},
      }
    },
  }
}

export function registerRelSignatureHelpProvider(monaco: typeof import('monaco-editor')): void {
  monaco.languages.registerSignatureHelpProvider(
    REL_LANGUAGE_ID,
    createRelSignatureHelpProvider(monaco)
  )
}
