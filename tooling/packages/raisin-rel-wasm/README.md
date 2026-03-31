# raisin-rel-wasm

WASM bindings for the Raisin Expression Language (REL). Provides real-time expression validation, evaluation, and autocomplete in the browser.

## License

This project is licensed under the Business Source License 1.1 (BSL-1.1). See the LICENSE file in the repository root for details.

## Features

- **Expression Validation** - Validate REL expressions with position-accurate errors
- **Expression Evaluation** - Evaluate expressions against a JSON context
- **AST Parsing** - Parse expressions to AST for tooling integration
- **AST Stringification** - Convert AST back to REL code
- **Autocomplete** - Context-aware completion suggestions for Monaco editor

## REL Expression Language

REL is a simple expression language for conditions, filters, and computed values:

```rel
// Property access
input.name
context.user.role

// Comparisons
input.age > 18
input.status == "active"

// Logical operators
input.enabled && context.hasPermission
input.type == "admin" || input.type == "moderator"

// Method calls
input.name.toLowerCase()
input.tags.contains("featured")
input.path.startsWith("/public/")

// Null handling
input.optional.isEmpty()
input.value.isNotEmpty()
```

## Building

Requires Rust toolchain with wasm32-unknown-unknown target:

```bash
# Install wasm-pack if not already installed
cargo install wasm-pack

# Build the WASM package
./build.sh
```

Output is generated in the `pkg/` directory.

## API

### Validation

```typescript
import { validate_expression } from 'raisin-rel-wasm';

const result = validate_expression('input.value > 10');
// { valid: true, errors: [] }

const invalid = validate_expression('input.value >');
// { valid: false, errors: [{ line: 1, column: 14, message: "...", severity: "error" }] }
```

### Evaluation

```typescript
import { evaluate_expression } from 'raisin-rel-wasm';

const result = evaluate_expression(
  'input.price * input.quantity',
  { input: { price: 10.50, quantity: 3 } }
);
// { success: true, value: 31.5, error: null }
```

### AST Operations

```typescript
import { parse_expression, stringify_expression } from 'raisin-rel-wasm';

// Parse to AST
const ast = parse_expression('input.name.toLowerCase()');
// { success: true, ast: {...}, error: null }

// Convert AST back to code
const code = stringify_expression(ast.ast);
// { success: true, code: "input.name.toLowerCase()", error: null }
```

### Autocomplete

```typescript
import { get_completions, get_method_completions } from 'raisin-rel-wasm';

// Get completions at cursor position
const completions = get_completions('input.', 6);

// Get all available methods
const methods = get_method_completions();
```

## Types

```typescript
interface ValidationResult {
  valid: boolean;
  errors: ValidationError[];
}

interface ValidationError {
  line: number;      // 1-based line number
  column: number;    // 1-based column number
  end_line: number;
  end_column: number;
  message: string;
  severity: 'error' | 'warning';
}

interface EvaluationResult {
  success: boolean;
  value: any | null;
  error: string | null;
}

interface ParseResult {
  success: boolean;
  ast: object | null;
  error: ValidationError | null;
}

interface CompletionItem {
  label: string;
  kind: 'keyword' | 'function' | 'variable' | 'operator' | 'method';
  insert_text: string;
  detail: string | null;
}
```

## Available Methods

### Universal Methods
| Method | Description |
|--------|-------------|
| `length()` | Get length (string, array, or object keys) |
| `isEmpty()` | Check if empty or null |
| `isNotEmpty()` | Check if not empty |

### String Methods
| Method | Description |
|--------|-------------|
| `contains(value)` | Check if string contains substring |
| `startsWith(prefix)` | Check if starts with prefix |
| `endsWith(suffix)` | Check if ends with suffix |
| `toLowerCase()` | Convert to lowercase |
| `toUpperCase()` | Convert to uppercase |
| `trim()` | Remove whitespace |
| `substring(start, end)` | Extract substring |

### Array Methods
| Method | Description |
|--------|-------------|
| `contains(element)` | Check if array contains element |
| `first()` | Get first element |
| `last()` | Get last element |
| `indexOf(element)` | Find element index (-1 if not found) |
| `join(separator)` | Join array into string |

### Path Methods
| Method | Description |
|--------|-------------|
| `parent()` | Get parent path |
| `ancestor(depth)` | Get ancestor at depth |
| `ancestorOf(path)` | Check if ancestor of path |
| `descendantOf(path)` | Check if descendant of path |
| `childOf(path)` | Check if direct child of path |
| `depth()` | Get hierarchy depth |

## Internal Dependencies

| Crate | Description |
|-------|-------------|
| `raisin-rel` | REL parser and evaluator |
