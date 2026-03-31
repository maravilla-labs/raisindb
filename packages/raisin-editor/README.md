# @raisindb/editor

Monaco-based code editors for RaisinDB with language-specific support.

## License

This project is licensed under the Business Source License 1.1 (BSL-1.1). See the LICENSE file in the repository root for details.

## Features

### Editor Components

- **CodeEditor** - Base Monaco editor component with RaisinDB theme
- **JavaScriptEditor** - JavaScript editor with RaisinDB API completions
- **StarlarkEditor** - Starlark/Python editor for serverless functions

### Language Support

- **JavaScript** - Full language support with RaisinDB-specific API completions
- **Starlark** - Custom language definition with syntax highlighting, tokenization, and RaisinDB function completions

### Themes

- **raisin-dark** - Dark theme matching the RaisinDB admin console design

## Installation

This is an internal package. Install from the monorepo:

```json
{
  "dependencies": {
    "@raisindb/editor": "file:../raisin-editor"
  }
}
```

## Usage

```tsx
import {
  CodeEditor,
  JavaScriptEditor,
  StarlarkEditor,
  registerRaisinDarkTheme
} from '@raisindb/editor';

// Register the theme once at app startup
registerRaisinDarkTheme(monaco);

// Use the editors
<JavaScriptEditor
  value={code}
  onChange={setCode}
  height="400px"
/>

<StarlarkEditor
  value={starlarkCode}
  onChange={setStarlarkCode}
  height="400px"
/>
```

## Exports

### Components

| Export | Description |
|--------|-------------|
| `CodeEditor` | Base Monaco editor with RaisinDB configuration |
| `JavaScriptEditor` | JavaScript editor with API completions |
| `StarlarkEditor` | Starlark/Python editor for functions |

### Themes

| Export | Description |
|--------|-------------|
| `registerRaisinDarkTheme` | Register the dark theme with Monaco |
| `RAISIN_DARK_THEME` | Theme name constant |

### Language Support

| Export | Description |
|--------|-------------|
| `registerRaisinJsCompletionProvider` | Register JavaScript completions |
| `getRaisinApiCompletions` | Get API completion items |
| `registerStarlarkLanguage` | Register Starlark language definition |
| `registerRaisinStarlarkCompletionProvider` | Register Starlark completions |
| `STARLARK_LANGUAGE_ID` | Starlark language ID constant |

## Internal Dependencies

| Package | Description |
|---------|-------------|
| `@raisindb/sql-wasm` | SQL validation (peer dependency for SQL editor features) |

## Peer Dependencies

- `react` ^18.3.1
- `react-dom` ^18.3.1
