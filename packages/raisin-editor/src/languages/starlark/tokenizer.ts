/**
 * Starlark Monarch tokenizer for syntax highlighting
 *
 * Defines tokens for keywords, builtins, strings, numbers, operators, etc.
 */

import type { languages } from 'monaco-editor'

/**
 * Monarch tokenizer for Starlark language
 */
export const starlarkTokenizer: languages.IMonarchLanguage = {
  defaultToken: '',
  tokenPostfix: '.starlark',

  keywords: [
    'def',
    'if',
    'elif',
    'else',
    'for',
    'in',
    'while',
    'break',
    'continue',
    'pass',
    'return',
    'and',
    'or',
    'not',
    'True',
    'False',
    'None',
    'lambda',
    'load',
  ],

  builtins: [
    'print',
    'range',
    'len',
    'str',
    'int',
    'bool',
    'list',
    'dict',
    'tuple',
    'type',
    'hasattr',
    'getattr',
    'fail',
    'struct',
    'enumerate',
    'zip',
    'sorted',
    'reversed',
    'min',
    'max',
    'sum',
    'all',
    'any',
    'abs',
    'repr',
  ],

  raisinGlobals: ['raisin'],

  operators: [
    '=',
    '==',
    '!=',
    '<',
    '<=',
    '>',
    '>=',
    '+',
    '-',
    '*',
    '/',
    '//',
    '%',
    '**',
    '+=',
    '-=',
    '*=',
    '/=',
    '//=',
    '%=',
    '&',
    '|',
    '^',
    '~',
    '<<',
    '>>',
  ],

  brackets: [
    { open: '{', close: '}', token: 'delimiter.curly' },
    { open: '[', close: ']', token: 'delimiter.square' },
    { open: '(', close: ')', token: 'delimiter.parenthesis' },
  ],

  tokenizer: {
    root: [
      // Whitespace
      [/\s+/, 'white'],

      // Comments
      [/#.*$/, 'comment'],

      // Multi-line strings (docstrings)
      [/"""/, 'string', '@multistring_double'],
      [/'''/, 'string', '@multistring_single'],

      // Strings
      [/"([^"\\]|\\.)*$/, 'string.invalid'], // Non-terminated string
      [/'([^'\\]|\\.)*$/, 'string.invalid'], // Non-terminated string
      [/"/, 'string', '@string_double'],
      [/'/, 'string', '@string_single'],

      // Numbers
      [/0[xX][0-9a-fA-F]+/, 'number.hex'],
      [/0[oO][0-7]+/, 'number.octal'],
      [/0[bB][01]+/, 'number.binary'],
      [/\d+\.\d*([eE][-+]?\d+)?/, 'number.float'],
      [/\d*\.\d+([eE][-+]?\d+)?/, 'number.float'],
      [/\d+[eE][-+]?\d+/, 'number.float'],
      [/\d+/, 'number'],

      // Function definitions
      [
        /\b(def)(\s+)([a-zA-Z_]\w*)/,
        ['keyword', 'white', 'function.declaration'],
      ],

      // Keywords
      [
        /\b(def|if|elif|else|for|in|while|break|continue|pass|return|and|or|not|True|False|None|lambda|load)\b/,
        'keyword',
      ],

      // Builtins
      [
        /\b(print|range|len|str|int|bool|list|dict|tuple|type|hasattr|getattr|fail|struct|enumerate|zip|sorted|reversed|min|max|sum|all|any|abs|repr)\b/,
        'predefined',
      ],

      // Raisin API global
      [/\braisin\b/, 'variable.predefined'],

      // Identifiers
      [/[a-zA-Z_]\w*/, 'identifier'],

      // Delimiters and operators
      [/[{}()\[\]]/, '@brackets'],
      [/[,.:;]/, 'delimiter'],
      [/[=<>!+\-*/%&|^~]+/, 'operator'],
    ],

    string_double: [
      [/[^\\"]+/, 'string'],
      [/\\./, 'string.escape'],
      [/"/, 'string', '@pop'],
    ],

    string_single: [
      [/[^\\']+/, 'string'],
      [/\\./, 'string.escape'],
      [/'/, 'string', '@pop'],
    ],

    multistring_double: [
      [/[^"]+/, 'string'],
      [/"""/, 'string', '@pop'],
      [/"/, 'string'],
    ],

    multistring_single: [
      [/[^']+/, 'string'],
      [/'''/, 'string', '@pop'],
      [/'/, 'string'],
    ],
  },
}
