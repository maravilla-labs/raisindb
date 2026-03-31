/**
 * REL (Raisin Expression Language) Monarch tokenizer for syntax highlighting
 */

import type { languages } from 'monaco-editor'

export const relMonarchTokenizer: languages.IMonarchLanguage = {
  defaultToken: 'text',
  tokenPostfix: '.rel',

  keywords: ['true', 'false', 'null', 'RELATES', 'VIA', 'DEPTH', 'DIRECTION', 'OUTGOING', 'INCOMING', 'ANY'],
  functions: ['contains', 'startsWith', 'endsWith'],

  brackets: [
    { open: '[', close: ']', token: 'delimiter.square' },
    { open: '(', close: ')', token: 'delimiter.parenthesis' },
    { open: '{', close: '}', token: 'delimiter.curly' },
  ],

  tokenizer: {
    root: [
      // Whitespace
      [/\s+/, 'white'],

      // RELATES keyword
      [/\b(RELATES)\b/i, 'keyword.relates'],

      // VIA, DEPTH, DIRECTION clauses
      [/\b(VIA|DEPTH|DIRECTION)\b/i, 'keyword.clause'],

      // Direction modifiers
      [/\b(OUTGOING|INCOMING|ANY)\b/i, 'keyword.modifier'],

      // Field access patterns (input.field, context.user.name)
      [/\b(input|context|node|auth|resource)\.[a-zA-Z_][\w.]*\b/, 'variable.field'],

      // Functions
      [/\b(contains|startsWith|endsWith)\b/, 'function'],

      // Keywords (true, false, null)
      [/\b(true|false)\b/, 'keyword.boolean'],
      [/\bnull\b/, 'keyword.null'],

      // Numbers (integers and floats)
      [/-?\d+(\.\d+)?([eE][+-]?\d+)?/, 'number'],

      // Strings - single quoted
      [/'([^'\\]|\\.)*$/, 'string.invalid'], // non-terminated
      [/'/, { token: 'string.quote', bracket: '@open', next: '@stringSingle' }],

      // Strings - double quoted
      [/"([^"\\]|\\.)*$/, 'string.invalid'], // non-terminated
      [/"/, { token: 'string.quote', bracket: '@open', next: '@stringDouble' }],

      // Logical operators
      [/&&/, 'keyword.combinator'],
      [/\|\|/, 'keyword.combinator'],
      [/!(?!=)/, 'operator.not'],

      // Comparison operators
      [/==|!=|>=|<=|>|</, 'operator.comparison'],

      // Arithmetic operators
      [/[+\-*/%]/, 'operator'],

      // Property access dot
      [/\./, 'delimiter'],

      // Brackets and delimiters
      [/[()]/, '@brackets'],
      [/[[\]]/, '@brackets'],
      [/[{}]/, '@brackets'],
      [/[,:]/, 'delimiter'],

      // Identifiers
      [/[a-zA-Z_][\w]*/, 'identifier'],
    ],

    stringSingle: [
      [/[^'\\]+/, 'string'],
      [/\\./, 'string.escape'],
      [/'/, { token: 'string.quote', bracket: '@close', next: '@pop' }],
    ],

    stringDouble: [
      [/[^"\\]+/, 'string'],
      [/\\./, 'string.escape'],
      [/"/, { token: 'string.quote', bracket: '@close', next: '@pop' }],
    ],
  },
}
