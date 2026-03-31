/**
 * REL (Raisin Expression Language) Monaco Editor integration
 *
 * Exports all components and utilities for REL language support in Monaco.
 */

export { RelEditor } from './RelEditor'
export type { RelEditorProps } from './RelEditor'

export {
  registerRelLanguage,
  getRelLanguageId,
  isRelRegistered,
  REL_LANGUAGE_ID,
  updateRelCompletionOptions,
} from './register'
export type { RelCompletionOptions } from './register'

export { relLanguageConfig } from './language-config'
export { relMonarchTokenizer } from './tokenizer'
export { createRelCompletionProvider, registerRelCompletionProvider } from './completion'
export { createRelHoverProvider, registerRelHoverProvider } from './hover'
export { createRelSignatureHelpProvider, registerRelSignatureHelpProvider } from './signature-help'
