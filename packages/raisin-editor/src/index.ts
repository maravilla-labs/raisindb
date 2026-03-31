/**
 * RaisinDB Editor Package
 *
 * Monaco-based code editors for RaisinDB with language-specific support.
 */

// Components
export { CodeEditor, JavaScriptEditor, StarlarkEditor } from './components'
export type { CodeEditorProps, JavaScriptEditorProps, StarlarkEditorProps } from './components'

// Themes
export { registerRaisinDarkTheme, RAISIN_DARK_THEME } from './themes'

// Languages - JavaScript
export { registerRaisinJsCompletionProvider, getRaisinApiCompletions } from './languages/javascript'
export type { RaisinApiCompletion } from './languages/javascript'

// Languages - Starlark/Python
export {
  registerStarlarkLanguage,
  registerRaisinStarlarkCompletionProvider,
  getRaisinStarlarkCompletions,
  STARLARK_LANGUAGE_ID,
} from './languages/starlark'
