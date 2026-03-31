# Architecture

## Design Philosophy

REL is designed as a simple, safe expression language optimized for security conditions. Key principles:

1. **Fail-Closed Security**: Errors result in `false` (deny access), never `true`
2. **Null-Safe**: Property access on null returns null, not errors
3. **Deterministic**: Same input always produces same output
4. **No Side Effects**: Expressions cannot modify state
5. **Async-Capable**: Graph traversal queries can be async

## Core Abstractions

### Expression AST

All expressions are represented as an `Expr` enum:

```rust
pub enum Expr {
    Literal(Literal),
    Variable(String),
    PropertyAccess { object: Box<Expr>, property: String },
    IndexAccess { object: Box<Expr>, index: Box<Expr> },
    BinaryOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
    UnaryOp { op: UnOp, expr: Box<Expr> },
    MethodCall { object: Box<Expr>, method: String, args: Vec<Expr> },
    Grouped(Box<Expr>),
    Relates { source, target, relation_types, min_depth, max_depth, direction },
}
```

### Value Types

Runtime values are represented as:

```rust
pub enum Value {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}
```

### EvalContext

The evaluation context holds variables accessible during evaluation:

```
┌─────────────────────────────────────────────────────┐
│                    EvalContext                       │
│                                                      │
│  variables: HashMap<String, Value>                   │
│                                                      │
│  Common variables in RLS context:                    │
│  ┌─────────────────────────────────────────────┐    │
│  │  "node" → {                                  │    │
│  │    "id": "doc123",                          │    │
│  │    "created_by": "user456",                 │    │
│  │    "path": "/content/blog/post1",           │    │
│  │    "status": "published",                   │    │
│  │    ...properties                            │    │
│  │  }                                          │    │
│  │                                              │    │
│  │  "auth" → {                                  │    │
│  │    "user_id": "user789",                    │    │
│  │    "local_user_id": "local123",             │    │
│  │    "roles": ["editor", "viewer"],           │    │
│  │    "groups": ["team-a"],                    │    │
│  │    ...                                      │    │
│  │  }                                          │    │
│  └─────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────┘
```

## Parsing Pipeline

```
Input String
     │
     ▼
┌─────────────────────────────────────────────────────┐
│                    Parser (nom)                      │
│                                                      │
│  1. Tokenize (whitespace, operators, identifiers)   │
│  2. Parse literals (strings, numbers, booleans)     │
│  3. Parse expressions with precedence               │
│  4. Handle method calls and property access         │
│  5. Parse RELATES expressions                       │
│                                                      │
│  Precedence (low to high):                          │
│  1. || (logical OR)                                 │
│  2. && (logical AND)                                │
│  3. ==, != (equality)                               │
│  4. <, >, <=, >= (comparison)                       │
│  5. !, - (unary)                                    │
│  6. ., [] (property/index access)                   │
│  7. () (method calls, grouping)                     │
└─────────────────────────────────────────────────────┘
     │
     ▼
Expr (AST)
```

## Evaluation Flow

### Synchronous Evaluation

```
Expr + EvalContext
     │
     ▼
┌─────────────────────────────────────────────────────┐
│                    Evaluator                         │
│                                                      │
│  Literal      → Value directly                      │
│  Variable     → ctx.get(name)                       │
│  PropertyAccess → evaluate(obj).get(prop)          │
│  IndexAccess  → evaluate(obj)[evaluate(idx)]       │
│  BinaryOp     → evaluate(left) op evaluate(right)  │
│  UnaryOp      → op evaluate(expr)                   │
│  MethodCall   → evaluate(obj).method(args)         │
│  Grouped      → evaluate(inner)                     │
│  Relates      → ERROR (requires async)             │
│                                                      │
│  Short-circuit evaluation:                          │
│  - AND (&&): if left is false, return false        │
│  - OR (||): if left is true, return true           │
└─────────────────────────────────────────────────────┘
     │
     ▼
Result<Value, EvalError>
```

### Async Evaluation (RELATES)

```
┌─────────────────────────────────────────────────────┐
│              Async Evaluator Pipeline                │
│                                                      │
│  1. Check if expression requires async:             │
│     requires_async(expr) → bool                     │
│                                                      │
│  2. If contains RELATES:                            │
│     ┌─────────────────────────────────────────┐     │
│     │         RelationResolver                │     │
│     │                                         │     │
│     │  async fn has_path(                     │     │
│     │    source_id, target_id,               │     │
│     │    relation_types,                     │     │
│     │    min_depth, max_depth,               │     │
│     │    direction                           │     │
│     │  ) -> Result<bool>                     │     │
│     └─────────────────────────────────────────┘     │
│                                                      │
│  3. Graph traversal (BFS/DFS) checks path exists   │
│                                                      │
│  4. Result combined with other conditions           │
└─────────────────────────────────────────────────────┘
```

## Null-Safe Semantics

REL follows JavaScript's optional chaining (`?.`) pattern:

```
┌─────────────────────────────────────────────────────┐
│                 Null-Safe Access                     │
│                                                      │
│  input.name         → if input is null → null       │
│  input.name.lower() → if name is null → null        │
│  input[0]           → if input is null → null       │
│                                                      │
│  This allows safe chaining without explicit checks: │
│                                                      │
│  Instead of:                                        │
│    input != null && input.meta != null              │
│      && input.meta.published == true                │
│                                                      │
│  Write:                                             │
│    input.meta.published == true                     │
│    (returns false if any part is null)             │
└─────────────────────────────────────────────────────┘
```

## Integration Points

### Row-Level Security (raisin-rocksdb)

```
┌─────────────────────────────────────────────────────┐
│               Permission Check Flow                  │
│                                                      │
│  1. ConditionEvaluator receives:                    │
│     - AuthContext (user, roles, groups)             │
│     - Node (properties, path, created_by)           │
│     - REL expression string                         │
│                                                      │
│  2. build_rel_context() creates EvalContext:        │
│     {                                               │
│       "node": { ...node properties },               │
│       "auth": { ...auth context }                   │
│     }                                               │
│                                                      │
│  3. raisin_rel::eval(expr, &ctx)                   │
│                                                      │
│  4. If error → false (fail-closed)                 │
│     If success → value.is_truthy()                 │
└─────────────────────────────────────────────────────┘
```

### Flow Runtime (raisin-flow-runtime)

```
┌─────────────────────────────────────────────────────┐
│              Decision Node Evaluation                │
│                                                      │
│  1. DecisionHandler receives FlowContext:           │
│     - input: JSON payload                           │
│     - variables: accumulated state                  │
│                                                      │
│  2. Build EvalContext from flow context:            │
│     {                                               │
│       "input": { ...flow input },                   │
│       ...variables                                  │
│     }                                               │
│                                                      │
│  3. Evaluate condition expression                   │
│                                                      │
│  4. Branch based on truthy/falsy result:            │
│     - true → follow yes_branch                      │
│     - false → follow no_branch                      │
└─────────────────────────────────────────────────────┘
```

## Error Handling

```
┌─────────────────────────────────────────────────────┐
│                   Error Types                        │
│                                                      │
│  ParseError (with position info):                   │
│  - SyntaxError { line, column, message }           │
│  - UnexpectedToken { line, column, expected, found }│
│  - UnexpectedEof { line, column }                  │
│  - InvalidNumber { line, column, value }           │
│  - UnterminatedString { line, column }             │
│                                                      │
│  EvalError:                                         │
│  - UndefinedVariable(name)                         │
│  - PropertyNotFound { property, value_type }       │
│  - IndexOutOfBounds { index, length }              │
│  - TypeError { operation, expected, actual }       │
│  - UnknownMethod(name)                             │
│  - WrongArgCount { function, expected, actual }    │
│  - GraphError(message)                             │
│                                                      │
│  RelError (combined):                               │
│  - Parse(ParseError)                               │
│  - Eval(EvalError)                                 │
└─────────────────────────────────────────────────────┘
```

## Module Structure

```
raisin-rel/
├── src/
│   ├── lib.rs           # Public API, re-exports
│   ├── ast/
│   │   ├── mod.rs       # AST exports
│   │   ├── expr.rs      # Expr, BinOp, UnOp, RelDirection
│   │   └── literal.rs   # Literal type
│   ├── parser/
│   │   ├── mod.rs       # Parser entry point
│   │   ├── common.rs    # Whitespace, identifiers
│   │   ├── literal.rs   # Literal parsing
│   │   └── expr.rs      # Expression parsing
│   ├── eval/
│   │   ├── mod.rs       # Eval exports
│   │   ├── context.rs   # EvalContext
│   │   ├── evaluator.rs # Sync evaluator
│   │   ├── async_evaluator.rs # Async evaluator
│   │   └── resolver.rs  # RelationResolver trait
│   ├── value.rs         # Value type
│   └── error.rs         # Error types
└── Cargo.toml
```

## Security Considerations

1. **No arbitrary code execution**: REL is pure expression evaluation
2. **Fail-closed**: Any error results in access denial
3. **Bounded complexity**: No loops or recursion
4. **Read-only**: Cannot modify context or external state
5. **Timeout-safe**: Evaluation is bounded (no infinite loops)
