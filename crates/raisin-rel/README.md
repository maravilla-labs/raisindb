# raisin-rel

Raisin Expression Language (REL) - a simple, safe expression language for evaluating conditions in RaisinDB.

## Overview

REL provides a lightweight expression language for defining conditions in:
- **Row-Level Security (RLS)** - Permission conditions like `node.created_by == auth.user_id`
- **Flow Runtime** - Decision nodes in workflow automation
- **Admin Console** - Client-side condition builder via WASM

## Expression Syntax

### Literals

```text
42              // Integer
3.14            // Float
'hello'         // String (single quotes)
"world"         // String (double quotes)
true, false     // Boolean
null            // Null
[1, 2, 3]       // Array
{key: 'value'}  // Object
```

### Operators

| Category | Operators |
|----------|-----------|
| Comparison | `==`, `!=`, `>`, `<`, `>=`, `<=` |
| Logical | `&&` (AND), `\|\|` (OR), `!` (NOT) |
| Arithmetic | `+`, `-`, `*`, `/`, `%` |
| Unary | `-` (negation) |

### Property Access

```text
input.value              // Property access
context.user.name        // Chained access
input.tags[0]            // Array index
data["key"]              // Object key access
```

### Methods

**Universal Methods** (work on String, Array, Object):
- `length()` - Get length
- `isEmpty()` - Check if empty
- `isNotEmpty()` - Check if not empty

**String Methods**:
- `contains(str)` - Check substring
- `startsWith(prefix)` - Check prefix
- `endsWith(suffix)` - Check suffix
- `toLowerCase()` - Convert to lowercase
- `toUpperCase()` - Convert to uppercase
- `trim()` - Trim whitespace
- `substring(start, end?)` - Extract substring

**Array Methods**:
- `contains(element)` - Check if element exists
- `first()` - Get first element
- `last()` - Get last element
- `indexOf(element)` - Find element index
- `join(separator?)` - Join elements

**Path Methods** (for hierarchical paths):
- `parent(levels?)` - Get parent path
- `ancestor(depth)` - Get ancestor at depth
- `depth()` - Get path depth
- `ancestorOf(path)` - Check if ancestor of
- `descendantOf(path)` - Check if descendant of
- `childOf(path)` - Check if direct child of

### Graph Relationships (RELATES)

For permission checks involving graph traversal:

```text
node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'
node.created_by RELATES auth.local_user_id VIA 'MANAGES' DEPTH 1..3
node.created_by RELATES auth.local_user_id VIA ['owns', 'manages'] DIRECTION OUTGOING
```

## Usage

### Basic Evaluation

```rust
use raisin_rel::{parse, evaluate, EvalContext, Value};
use std::collections::HashMap;

// Parse an expression
let expr = parse("input.value > 10 && input.status == 'active'").unwrap();

// Create evaluation context
let mut input = HashMap::new();
input.insert("value".to_string(), Value::Integer(42));
input.insert("status".to_string(), Value::String("active".to_string()));

let mut ctx = EvalContext::new();
ctx.set("input", Value::Object(input));

// Evaluate
let result = evaluate(&expr, &ctx).unwrap();
assert_eq!(result, Value::Boolean(true));
```

### From JSON

```rust
use raisin_rel::{eval, EvalContext};

let json = serde_json::json!({
    "input": {
        "value": 42,
        "status": "active"
    }
});

let ctx = EvalContext::from_json(json).unwrap();
let result = eval("input.value > 10", &ctx).unwrap();
```

### Method Chaining

```rust
// Methods can be chained
eval("name.trim().toLowerCase().contains('test')", &ctx)
```

### Null-Safe Access

REL provides null-safe property and method access (like JavaScript's `?.`):

```rust
// Returns null instead of error if input.name is null
eval("input.name.toLowerCase()", &ctx)

// Short-circuit evaluation with &&
eval("input.meta && input.meta.published", &ctx)
```

### Async Evaluation (RELATES)

For expressions with graph relationships:

```rust
use raisin_rel::{parse, evaluate_async, requires_async, EvalContext};
use raisin_rel::eval::RelationResolver;

let expr = parse("node.created_by RELATES auth.user_id VIA 'MANAGES'").unwrap();

// Check if async evaluation is needed
if requires_async(&expr) {
    let resolver: &dyn RelationResolver = /* your graph resolver */;
    let result = evaluate_async(&expr, &ctx, resolver).await?;
}
```

## Project Usage

### Row-Level Security (raisin-rocksdb)

REL powers permission conditions in the security layer:

```rust
// raisin-rocksdb/src/security/condition_evaluator.rs
let evaluator = ConditionEvaluator::new(&auth_context);
evaluator.evaluate_rel_expression(
    "node.created_by == auth.user_id || auth.roles.contains('admin')",
    &node
)
```

### Flow Runtime (raisin-flow-runtime)

REL evaluates decision node conditions:

```yaml
# Flow definition
nodes:
  - id: check-priority
    type: decision
    properties:
      condition: "input.priority >= 5 || input.urgent == true"
      yes_branch: "urgent-handler"
      no_branch: "normal-handler"
```

### Admin Console (WASM)

REL is compiled to WASM for client-side condition building and validation in the admin console's `ConditionBuilder` component.

## Features

- **Null-safe**: Property/method access on null returns null (no errors)
- **Short-circuit**: `&&` and `||` short-circuit evaluation
- **Fail-closed**: Parse/eval errors return false in security contexts
- **Serializable**: AST is JSON-serializable via serde
- **Async-capable**: RELATES expressions support async graph queries

## Components

| Module | Description |
|--------|-------------|
| `ast` | Expression AST nodes (Expr, BinOp, UnOp, etc.) |
| `parser` | Expression parser using nom |
| `eval` | Sync/async evaluators and RelationResolver trait |
| `value` | Runtime value types |
| `error` | ParseError, EvalError, RelError types |

## Example Expressions

```text
// Basic comparisons
input.value > 10
input.status == 'active'

// Arithmetic
input.user.age + 5
price * quantity
total / count
score % 10
'hello' + ' ' + 'world'    // String concatenation

// Logical combinations
input.value > 10 && input.status == 'active'
input.priority >= 5 || input.urgent == true

// Method calls
input.name.contains('test')
input.tags.contains('important')
auth.roles.contains('editor')

// Path operations
node.path.descendantOf('/content/blog')
node.path.depth() <= 3

// Graph relationships
node.created_by RELATES auth.local_user_id VIA 'MANAGES' DEPTH 1..3
```

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
