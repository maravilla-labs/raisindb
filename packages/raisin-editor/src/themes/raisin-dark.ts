/**
 * RaisinDB dark theme for Monaco Editor
 */

import type { Monaco } from '@monaco-editor/react'

export const RAISIN_DARK_THEME = 'raisin-dark'

/**
 * Register the RaisinDB dark theme with Monaco Editor
 */
export function registerRaisinDarkTheme(monaco: Monaco): void {
  monaco.editor.defineTheme(RAISIN_DARK_THEME, {
    base: 'vs-dark',
    inherit: true,
    rules: [
      // JavaScript/TypeScript tokens
      { token: 'keyword', foreground: 'c586c0' },
      { token: 'keyword.control', foreground: 'c586c0' },
      { token: 'storage', foreground: '569cd6' },
      { token: 'storage.type', foreground: '569cd6' },
      { token: 'storage.modifier', foreground: '569cd6' },
      { token: 'type', foreground: '4ec9b0' },
      { token: 'type.identifier', foreground: '4ec9b0' },
      { token: 'function', foreground: 'dcdcaa' },
      { token: 'function.name', foreground: 'dcdcaa' },
      { token: 'variable', foreground: '9cdcfe' },
      { token: 'variable.predefined', foreground: '4fc1ff' },
      { token: 'parameter', foreground: '9cdcfe' },
      { token: 'property', foreground: '9cdcfe' },
      { token: 'string', foreground: 'ce9178' },
      { token: 'string.escape', foreground: 'd7ba7d' },
      { token: 'number', foreground: 'b5cea8' },
      { token: 'comment', foreground: '6a9955' },
      { token: 'comment.doc', foreground: '608b4e' },
      { token: 'operator', foreground: 'd4d4d4' },
      { token: 'delimiter', foreground: 'd4d4d4' },
      { token: 'delimiter.bracket', foreground: 'ffd700' },

      // Raisin API specific tokens (for raisin.* highlighting)
      { token: 'raisin.namespace', foreground: '4ec9b0', fontStyle: 'bold' },
      { token: 'raisin.method', foreground: 'dcdcaa' },
      { token: 'raisin.property', foreground: '9cdcfe' },
    ],
    colors: {
      'editor.background': '#1a1a2e',
      'editor.foreground': '#d4d4d4',
      'editor.lineHighlightBackground': '#2a2a4e',
      'editor.selectionBackground': '#264f78',
      'editorCursor.foreground': '#569cd6',
      'editorLineNumber.foreground': '#858585',
      'editorLineNumber.activeForeground': '#c6c6c6',
      'editorIndentGuide.background1': '#404040',
      'editorIndentGuide.activeBackground1': '#707070',
    },
  })
}
