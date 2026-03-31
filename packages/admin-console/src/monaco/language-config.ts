/**
 * RaisinDB SQL language configuration for Monaco Editor
 *
 * This defines the language ID and configuration options for
 * bracket matching, comments, and auto-closing pairs.
 */

import type { languages } from 'monaco-editor'

export const LANGUAGE_ID = 'raisinsql'

export const languageConfig: languages.LanguageConfiguration = {
  comments: {
    lineComment: '--',
    blockComment: ['/*', '*/'],
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
    { open: "'", close: "'", notIn: ['string', 'comment'] },
    { open: '"', close: '"', notIn: ['string', 'comment'] },
  ],
  surroundingPairs: [
    { open: '{', close: '}' },
    { open: '[', close: ']' },
    { open: '(', close: ')' },
    { open: "'", close: "'" },
    { open: '"', close: '"' },
  ],
  folding: {
    markers: {
      start: /^\s*--\s*#?region\b/,
      end: /^\s*--\s*#?endregion\b/,
    },
  },
}
