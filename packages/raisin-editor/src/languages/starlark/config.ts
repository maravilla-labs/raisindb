/**
 * Starlark language configuration for Monaco Editor
 *
 * Defines comments, brackets, auto-closing pairs, and indentation rules.
 */

import type { languages } from 'monaco-editor'

export const STARLARK_LANGUAGE_ID = 'starlark'

/**
 * Starlark language configuration
 */
export const starlarkLanguageConfig: languages.LanguageConfiguration = {
  comments: {
    lineComment: '#',
  },
  brackets: [
    ['{', '}'],
    ['[', ']'],
    ['(', ')'],
  ],
  autoClosingPairs: [
    { open: '{', close: '}' },
    { open: '[', close: ']' },
    { open: '(', close: ')' },
    { open: '"', close: '"', notIn: ['string'] },
    { open: "'", close: "'", notIn: ['string'] },
  ],
  surroundingPairs: [
    { open: '{', close: '}' },
    { open: '[', close: ']' },
    { open: '(', close: ')' },
    { open: '"', close: '"' },
    { open: "'", close: "'" },
  ],
  indentationRules: {
    increaseIndentPattern: /^\s*(def|if|elif|else|for|while).*:\s*$/,
    decreaseIndentPattern: /^\s*(elif|else)\b/,
  },
  folding: {
    markers: {
      start: /^\s*#\s*region\b/,
      end: /^\s*#\s*endregion\b/,
    },
  },
  onEnterRules: [
    {
      // After a line ending with `:`, indent
      beforeText: /:\s*$/,
      action: { indentAction: 1 }, // Indent
    },
  ],
}
