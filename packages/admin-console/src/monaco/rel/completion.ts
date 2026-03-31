/**
 * REL (Raisin Expression Language) completion provider for Monaco Editor
 */

import type { languages, editor, Position, CancellationToken } from 'monaco-editor'
import { REL_LANGUAGE_ID } from './language-config'

// Method completions for method-chaining syntax (e.g., input.name.contains('test'))
const methodCompletions = [
  // Universal methods (work on String, Array, Object)
  {
    label: 'length',
    kind: 1, // Method
    insertText: 'length()',
    detail: '.length() -> number',
    documentation: 'Get length of string, array, or number of object keys.',
  },
  {
    label: 'isEmpty',
    kind: 1, // Method
    insertText: 'isEmpty()',
    detail: '.isEmpty() -> boolean',
    documentation: 'Check if empty (empty string, empty array, empty object, or null).',
  },
  {
    label: 'isNotEmpty',
    kind: 1, // Method
    insertText: 'isNotEmpty()',
    detail: '.isNotEmpty() -> boolean',
    documentation: 'Check if not empty and not null.',
  },

  // Polymorphic methods (String or Array)
  {
    label: 'contains',
    kind: 1, // Method
    insertText: 'contains(${1:value})',
    insertTextRules: 4, // InsertAsSnippet
    detail: '.contains(value) -> boolean',
    documentation: 'String: check if contains substring. Array: check if contains element.',
  },

  // String methods
  {
    label: 'startsWith',
    kind: 1, // Method
    insertText: 'startsWith(${1:prefix})',
    insertTextRules: 4,
    detail: '.startsWith(prefix) -> boolean',
    documentation: 'Check if string starts with the given prefix.',
  },
  {
    label: 'endsWith',
    kind: 1, // Method
    insertText: 'endsWith(${1:suffix})',
    insertTextRules: 4,
    detail: '.endsWith(suffix) -> boolean',
    documentation: 'Check if string ends with the given suffix.',
  },
  {
    label: 'toLowerCase',
    kind: 1, // Method
    insertText: 'toLowerCase()',
    detail: '.toLowerCase() -> string',
    documentation: 'Convert string to lowercase.',
  },
  {
    label: 'toUpperCase',
    kind: 1, // Method
    insertText: 'toUpperCase()',
    detail: '.toUpperCase() -> string',
    documentation: 'Convert string to uppercase.',
  },
  {
    label: 'trim',
    kind: 1, // Method
    insertText: 'trim()',
    detail: '.trim() -> string',
    documentation: 'Remove leading and trailing whitespace.',
  },
  {
    label: 'substring',
    kind: 1, // Method
    insertText: 'substring(${1:start}, ${2:end})',
    insertTextRules: 4,
    detail: '.substring(start, end?) -> string',
    documentation: 'Extract substring from start index to end index (optional).',
  },

  // Array methods
  {
    label: 'first',
    kind: 1, // Method
    insertText: 'first()',
    detail: '.first() -> any',
    documentation: 'Get first element of array (null if empty).',
  },
  {
    label: 'last',
    kind: 1, // Method
    insertText: 'last()',
    detail: '.last() -> any',
    documentation: 'Get last element of array (null if empty).',
  },
  {
    label: 'indexOf',
    kind: 1, // Method
    insertText: 'indexOf(${1:element})',
    insertTextRules: 4,
    detail: '.indexOf(element) -> number',
    documentation: 'Get index of element in array (-1 if not found).',
  },
  {
    label: 'join',
    kind: 1, // Method
    insertText: 'join(${1:separator})',
    insertTextRules: 4,
    detail: '.join(separator?) -> string',
    documentation: 'Join array elements into a string with optional separator.',
  },

  // Path methods
  {
    label: 'parent',
    kind: 1, // Method
    insertText: 'parent()',
    detail: '.parent(n?) -> string',
    documentation: 'Get parent path. Optional n for number of levels up (default 1).',
  },
  {
    label: 'ancestor',
    kind: 1, // Method
    insertText: 'ancestor(${1:depth})',
    insertTextRules: 4,
    detail: '.ancestor(depth) -> string',
    documentation: 'Get ancestor at absolute depth from root.',
  },
  {
    label: 'ancestorOf',
    kind: 1, // Method
    insertText: 'ancestorOf(${1:path})',
    insertTextRules: 4,
    detail: '.ancestorOf(path) -> boolean',
    documentation: 'Check if this path is an ancestor of the given path.',
  },
  {
    label: 'descendantOf',
    kind: 1, // Method
    insertText: 'descendantOf(${1:path})',
    insertTextRules: 4,
    detail: '.descendantOf(path) -> boolean',
    documentation: 'Check if this path is a descendant of the given path.',
  },
  {
    label: 'childOf',
    kind: 1, // Method
    insertText: 'childOf(${1:path})',
    insertTextRules: 4,
    detail: '.childOf(path) -> boolean',
    documentation: 'Check if this path is a direct child of the given path.',
  },
  {
    label: 'depth',
    kind: 1, // Method
    insertText: 'depth()',
    detail: '.depth() -> number',
    documentation: 'Get hierarchy depth of path.',
  },
]

const contextCompletions = [
  {
    label: 'input',
    kind: 5, // Variable
    insertText: 'input.',
    detail: 'Input data',
    documentation: 'Access the input data object. Use dot notation to access fields.',
  },
  {
    label: 'context',
    kind: 5, // Variable
    insertText: 'context.',
    detail: 'Execution context',
    documentation: 'Access the execution context object.',
  },
]

const keywordCompletions = [
  {
    label: 'true',
    kind: 13, // Keyword
    insertText: 'true',
    detail: 'Boolean true',
    documentation: 'Boolean literal representing true.',
  },
  {
    label: 'false',
    kind: 13, // Keyword
    insertText: 'false',
    detail: 'Boolean false',
    documentation: 'Boolean literal representing false.',
  },
  {
    label: 'null',
    kind: 13, // Keyword
    insertText: 'null',
    detail: 'Null value',
    documentation: 'Represents an absence of value.',
  },
  {
    label: 'RELATES',
    kind: 13, // Keyword
    insertText: 'RELATES',
    detail: 'Graph relationship check',
    documentation: 'Check if two nodes are related through a graph relationship.',
  },
  {
    label: 'VIA',
    kind: 13, // Keyword
    insertText: 'VIA',
    detail: 'Relationship type filter',
    documentation: 'Specify which relationship types to traverse in a RELATES check.',
  },
  {
    label: 'DEPTH',
    kind: 13, // Keyword
    insertText: 'DEPTH',
    detail: 'Relationship depth',
    documentation: 'Specify the minimum and maximum depth for relationship traversal.',
  },
  {
    label: 'DIRECTION',
    kind: 13, // Keyword
    insertText: 'DIRECTION',
    detail: 'Relationship direction',
    documentation: 'Specify the direction of relationship traversal (OUTGOING, INCOMING, or ANY).',
  },
  {
    label: 'OUTGOING',
    kind: 13, // Keyword
    insertText: 'OUTGOING',
    detail: 'Outgoing direction',
    documentation: 'Traverse only outgoing relationships.',
  },
  {
    label: 'INCOMING',
    kind: 13, // Keyword
    insertText: 'INCOMING',
    detail: 'Incoming direction',
    documentation: 'Traverse only incoming relationships.',
  },
  {
    label: 'ANY',
    kind: 13, // Keyword
    insertText: 'ANY',
    detail: 'Any direction',
    documentation: 'Traverse relationships in any direction.',
  },
]

const operatorCompletions = [
  {
    label: '==',
    kind: 11, // Operator
    insertText: '== ',
    detail: 'Equals',
    documentation: 'Equality comparison.',
  },
  {
    label: '!=',
    kind: 11, // Operator
    insertText: '!= ',
    detail: 'Not equals',
    documentation: 'Inequality comparison.',
  },
  {
    label: '>',
    kind: 11, // Operator
    insertText: '> ',
    detail: 'Greater than',
    documentation: 'Greater than comparison.',
  },
  {
    label: '<',
    kind: 11, // Operator
    insertText: '< ',
    detail: 'Less than',
    documentation: 'Less than comparison.',
  },
  {
    label: '>=',
    kind: 11, // Operator
    insertText: '>= ',
    detail: 'Greater or equal',
    documentation: 'Greater than or equal comparison.',
  },
  {
    label: '<=',
    kind: 11, // Operator
    insertText: '<= ',
    detail: 'Less or equal',
    documentation: 'Less than or equal comparison.',
  },
  {
    label: '&&',
    kind: 11, // Operator
    insertText: '&& ',
    detail: 'Logical AND',
    documentation: 'Returns true if both operands are true.',
  },
  {
    label: '||',
    kind: 11, // Operator
    insertText: '|| ',
    detail: 'Logical OR',
    documentation: 'Returns true if either operand is true.',
  },
]

export interface RelCompletionOptions {
  /** Available fields for input./context. completion */
  availableFields?: string[]
  /** Dynamic relation types from raisin:access_control/relation-types/ */
  relationTypes?: string[]
}

// Mutable options store for dynamic updates (since language registration only happens once)
let currentOptions: RelCompletionOptions = {}

/**
 * Update the completion options dynamically after registration.
 * Use this to provide dynamic relation types loaded from the API.
 */
export function updateRelCompletionOptions(options: Partial<RelCompletionOptions>): void {
  currentOptions = { ...currentOptions, ...options }
}

/**
 * Get the current completion options
 */
export function getRelCompletionOptions(): RelCompletionOptions {
  return currentOptions
}

export function createRelCompletionProvider(
  monaco: typeof import('monaco-editor'),
  options?: RelCompletionOptions
): languages.CompletionItemProvider {
  // Initialize with provided options
  if (options) {
    currentOptions = { ...currentOptions, ...options }
  }
  return {
    triggerCharacters: ['.', ' '],

    provideCompletionItems(
      model: editor.ITextModel,
      position: Position,
      _context: languages.CompletionContext,
      _token: CancellationToken
    ): languages.ProviderResult<languages.CompletionList> {
      const word = model.getWordUntilPosition(position)
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      }

      // Get text before cursor to determine context
      const textBefore = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      })

      const suggestions: languages.CompletionItem[] = []

      // After any dot - suggest both fields and methods
      // Methods are always available after a dot (null-safe chaining)
      if (textBefore.match(/\.\s*$/)) {
        // Suggest methods first
        for (const method of methodCompletions) {
          suggestions.push({
            label: method.label,
            kind: monaco.languages.CompletionItemKind.Method,
            insertText: method.insertText,
            insertTextRules: method.insertTextRules
              ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
              : undefined,
            range,
            detail: method.detail,
            documentation: method.documentation,
          })
        }

        // After input. or context. - also suggest fields
        if (textBefore.match(/\b(input|context)\.\s*$/)) {
          const { availableFields } = currentOptions
          if (availableFields) {
            for (const field of availableFields) {
              suggestions.push({
                label: field,
                kind: monaco.languages.CompletionItemKind.Field,
                insertText: field,
                range,
                detail: 'Field',
                documentation: `Access the ${field} field.`,
              })
            }
          }
          // Also suggest common field names
          const commonFields = ['value', 'status', 'type', 'name', 'id', 'data', 'path', 'node']
          for (const field of commonFields) {
            if (!availableFields?.includes(field)) {
              suggestions.push({
                label: field,
                kind: monaco.languages.CompletionItemKind.Field,
                insertText: field,
                range,
                detail: 'Common field',
                documentation: `Access the ${field} field.`,
              })
            }
          }
        }

        return { suggestions }
      }

      // After field path - suggest RELATES keyword
      if (textBefore.match(/[\w.]+\s*$/)) {
        suggestions.push({
          label: 'RELATES',
          kind: monaco.languages.CompletionItemKind.Keyword,
          insertText: 'RELATES ${1:target} VIA [${2:RELATION_TYPE}] DEPTH ${3:1} ${4:1} DIRECTION ${5:ANY}',
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          range,
          detail: 'Graph relationship check',
          documentation: 'Check if two nodes are related through a graph relationship.',
        })
      }

      // After VIA - suggest relation type patterns
      if (textBefore.match(/\bVIA\s*$/i)) {
        const { relationTypes } = currentOptions
        // Use dynamic relation types if provided, otherwise fall back to common defaults
        const availableRelations = relationTypes && relationTypes.length > 0
          ? relationTypes
          : ['FRIENDS_WITH', 'MEMBER_OF', 'CREATED_BY', 'OWNS', 'MANAGES']

        for (const rel of availableRelations) {
          suggestions.push({
            label: rel,
            kind: monaco.languages.CompletionItemKind.Constant,
            insertText: `'${rel}'`,
            range,
            detail: relationTypes?.includes(rel) ? 'Configured relation type' : 'Common relation type',
            documentation: `Filter by ${rel} relationship type.`,
          })
        }

        // Also suggest array syntax for multiple types
        suggestions.push({
          label: '[multiple types]',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "['${1:TYPE1}', '${2:TYPE2}']",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          range,
          detail: 'Multiple relation types',
          documentation: 'Specify multiple relationship types to traverse.',
        })
      }

      // After DIRECTION - suggest direction values
      if (textBefore.match(/\bDIRECTION\s*$/i)) {
        const directions = [
          { label: 'ANY', detail: 'Any direction' },
          { label: 'OUTGOING', detail: 'Outgoing direction →' },
          { label: 'INCOMING', detail: 'Incoming direction ←' },
        ]
        for (const dir of directions) {
          suggestions.push({
            label: dir.label,
            kind: monaco.languages.CompletionItemKind.Keyword,
            insertText: dir.label,
            range,
            detail: dir.detail,
            documentation: `Traverse relationships in ${dir.label.toLowerCase()} direction.`,
          })
        }
      }

      // After a value or identifier - suggest operators
      if (textBefore.match(/[\w'")\]]\s*$/)) {
        for (const op of operatorCompletions) {
          suggestions.push({
            label: op.label,
            kind: monaco.languages.CompletionItemKind.Operator,
            insertText: op.insertText,
            range,
            detail: op.detail,
            documentation: op.documentation,
          })
        }
      }

      // General completions - context roots, keywords (no standalone functions in method-chaining syntax)
      for (const ctx of contextCompletions) {
        suggestions.push({
          label: ctx.label,
          kind: monaco.languages.CompletionItemKind.Variable,
          insertText: ctx.insertText,
          range,
          detail: ctx.detail,
          documentation: ctx.documentation,
        })
      }

      for (const kw of keywordCompletions) {
        suggestions.push({
          label: kw.label,
          kind: monaco.languages.CompletionItemKind.Keyword,
          insertText: kw.insertText,
          range,
          detail: kw.detail,
          documentation: kw.documentation,
        })
      }

      return { suggestions }
    },
  }
}

export function registerRelCompletionProvider(
  monaco: typeof import('monaco-editor'),
  options?: RelCompletionOptions
): void {
  monaco.languages.registerCompletionItemProvider(
    REL_LANGUAGE_ID,
    createRelCompletionProvider(monaco, options)
  )
}
