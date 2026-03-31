/**
 * REL (Raisin Expression Language) hover provider for Monaco Editor
 */

import type { languages, editor, Position, CancellationToken } from 'monaco-editor'
import { REL_LANGUAGE_ID } from './language-config'

const hoverDocs = new Map<string, { description: string; syntax?: string; example?: string }>([
  // Methods - Universal
  ['length', {
    description: 'Get length of string, array, or number of object keys.',
    syntax: '.length() -> number',
    example: "input.name.length() > 10",
  }],
  ['isempty', {
    description: 'Check if empty (empty string, empty array, empty object, or null).',
    syntax: '.isEmpty() -> boolean',
    example: "input.name.isEmpty()",
  }],
  ['isnotempty', {
    description: 'Check if not empty and not null.',
    syntax: '.isNotEmpty() -> boolean',
    example: "input.name.isNotEmpty()",
  }],

  // Methods - String/Array (polymorphic)
  ['contains', {
    description: 'String: check if contains substring. Array: check if contains element.',
    syntax: '.contains(value) -> boolean',
    example: "input.name.contains('test')",
  }],

  // Methods - String
  ['startswith', {
    description: 'Check if string starts with the given prefix.',
    syntax: '.startsWith(prefix) -> boolean',
    example: "input.path.startsWith('/api')",
  }],
  ['endswith', {
    description: 'Check if string ends with the given suffix.',
    syntax: '.endsWith(suffix) -> boolean',
    example: "input.file.endsWith('.txt')",
  }],
  ['tolowercase', {
    description: 'Convert string to lowercase.',
    syntax: '.toLowerCase() -> string',
    example: "input.name.toLowerCase().contains('admin')",
  }],
  ['touppercase', {
    description: 'Convert string to uppercase.',
    syntax: '.toUpperCase() -> string',
    example: "input.code.toUpperCase() == 'ABC'",
  }],
  ['trim', {
    description: 'Remove leading and trailing whitespace.',
    syntax: '.trim() -> string',
    example: "input.text.trim().isNotEmpty()",
  }],
  ['substring', {
    description: 'Extract substring from start index to optional end index.',
    syntax: '.substring(start, end?) -> string',
    example: "input.id.substring(0, 3) == 'PRE'",
  }],

  // Methods - Array
  ['first', {
    description: 'Get first element of array (null if empty).',
    syntax: '.first() -> any',
    example: "input.tags.first() == 'important'",
  }],
  ['last', {
    description: 'Get last element of array (null if empty).',
    syntax: '.last() -> any',
    example: "input.history.last()",
  }],
  ['indexof', {
    description: 'Get index of element in array (-1 if not found).',
    syntax: '.indexOf(element) -> number',
    example: "input.tags.indexOf('admin') >= 0",
  }],
  ['join', {
    description: 'Join array elements into a string with optional separator.',
    syntax: '.join(separator?) -> string',
    example: "input.tags.join(', ')",
  }],

  // Methods - Path
  ['parent', {
    description: 'Get parent path. Optional n for number of levels up.',
    syntax: '.parent(n?) -> string',
    example: "input.node.path.parent() == '/content/blog'",
  }],
  ['ancestor', {
    description: 'Get ancestor at absolute depth from root.',
    syntax: '.ancestor(depth) -> string',
    example: "input.path.ancestor(2) == '/content/articles'",
  }],
  ['ancestorof', {
    description: 'Check if this path is an ancestor of the given path.',
    syntax: '.ancestorOf(path) -> boolean',
    example: "input.path.ancestorOf('/content/blog/post1')",
  }],
  ['descendantof', {
    description: 'Check if this path is a descendant of the given path.',
    syntax: '.descendantOf(path) -> boolean',
    example: "input.node.path.descendantOf('/content')",
  }],
  ['childof', {
    description: 'Check if this path is a direct child of the given path.',
    syntax: '.childOf(path) -> boolean',
    example: "input.path.childOf('/content/blog')",
  }],
  ['depth', {
    description: 'Get hierarchy depth of path.',
    syntax: '.depth() -> number',
    example: "input.node.path.depth() > 3",
  }],

  // Keywords
  ['true', {
    description: 'Boolean literal representing true.',
    syntax: 'true',
    example: "input.active == true",
  }],
  ['false', {
    description: 'Boolean literal representing false.',
    syntax: 'false',
    example: "input.disabled == false",
  }],
  ['null', {
    description: 'Represents an absence of value.',
    syntax: 'null',
    example: "input.optional != null",
  }],
  // Context roots
  ['input', {
    description: 'Access the input data object passed to the expression.',
    syntax: 'input.fieldName',
    example: "input.value > 10",
  }],
  ['context', {
    description: 'Access the execution context object with additional variables.',
    syntax: 'context.variableName',
    example: "context.user.role == 'admin'",
  }],
])

const operatorDocs = new Map<string, { description: string; example: string }>([
  ['==', { description: 'Equality comparison. Returns true if both values are equal.', example: "input.status == 'active'" }],
  ['!=', { description: 'Inequality comparison. Returns true if values are different.', example: "input.type != 'draft'" }],
  ['>', { description: 'Greater than comparison.', example: 'input.count > 10' }],
  ['<', { description: 'Less than comparison.', example: 'input.priority < 5' }],
  ['>=', { description: 'Greater than or equal comparison.', example: 'input.score >= 80' }],
  ['<=', { description: 'Less than or equal comparison.', example: 'input.rank <= 3' }],
  ['&&', { description: 'Logical AND. Returns true if both operands are true.', example: "input.active == true && input.verified == true" }],
  ['||', { description: 'Logical OR. Returns true if either operand is true.', example: "input.role == 'admin' || input.role == 'moderator'" }],
  ['!', { description: 'Logical NOT. Negates a boolean value.', example: '!input.disabled' }],
])

export function createRelHoverProvider(
  _monaco: typeof import('monaco-editor')
): languages.HoverProvider {
  return {
    provideHover(
      model: editor.ITextModel,
      position: Position,
      _token: CancellationToken
    ): languages.ProviderResult<languages.Hover> {
      const word = model.getWordAtPosition(position)
      if (!word) {
        // Check for operators
        const line = model.getLineContent(position.lineNumber)
        const col = position.column - 1

        // Check for two-character operators
        const twoChar = line.substring(col - 1, col + 1)
        const opDoc = operatorDocs.get(twoChar)
        if (opDoc) {
          return {
            contents: [{
              value: [
                `**${twoChar}** \`[Operator]\``,
                '',
                opDoc.description,
                '',
                '**Example:**',
                '```rel',
                opDoc.example,
                '```',
              ].join('\n'),
            }],
            range: {
              startLineNumber: position.lineNumber,
              endLineNumber: position.lineNumber,
              startColumn: col,
              endColumn: col + 2,
            },
          }
        }

        // Check for single-character operators
        const oneChar = line.charAt(col)
        const oneCharDoc = operatorDocs.get(oneChar)
        if (oneCharDoc) {
          return {
            contents: [{
              value: [
                `**${oneChar}** \`[Operator]\``,
                '',
                oneCharDoc.description,
                '',
                '**Example:**',
                '```rel',
                oneCharDoc.example,
                '```',
              ].join('\n'),
            }],
            range: {
              startLineNumber: position.lineNumber,
              endLineNumber: position.lineNumber,
              startColumn: col + 1,
              endColumn: col + 2,
            },
          }
        }

        return null
      }

      const wordLower = word.word.toLowerCase()
      const doc = hoverDocs.get(wordLower)

      if (doc) {
        const contents: string[] = [
          `**${word.word}** \`[REL]\``,
          '',
          doc.description,
        ]

        if (doc.syntax) {
          contents.push('', '**Syntax:**', '```rel', doc.syntax, '```')
        }

        if (doc.example) {
          contents.push('', '**Example:**', '```rel', doc.example, '```')
        }

        return {
          contents: [{ value: contents.join('\n') }],
          range: {
            startLineNumber: position.lineNumber,
            endLineNumber: position.lineNumber,
            startColumn: word.startColumn,
            endColumn: word.endColumn,
          },
        }
      }

      return null
    },
  }
}

export function registerRelHoverProvider(monaco: typeof import('monaco-editor')): void {
  monaco.languages.registerHoverProvider(REL_LANGUAGE_ID, createRelHoverProvider(monaco))
}
