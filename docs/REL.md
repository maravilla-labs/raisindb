# REL - Raisin Expression Language

REL (Raisin Expression Language) is a simple yet powerful expression language for evaluating conditions in RaisinDB. It's designed for use in triggers, flow conditions, and rule-based logic.

## Quick Start

```rel
// Simple comparison
input.value > 10

// String method
input.name.contains('admin')

// Combined conditions
input.status == 'active' && input.priority >= 5

// Path operations
input.node.path.descendantOf('/content/blog')

// Chained methods
input.text.trim().toLowerCase().startsWith('hello')
```

## Expression Syntax

### Literals

| Type | Examples |
|------|----------|
| Strings | `'hello'`, `"world"` |
| Numbers | `42`, `3.14`, `-10` |
| Booleans | `true`, `false` |
| Null | `null` |
| Arrays | `[1, 2, 3]`, `['a', 'b']` |
| Objects | `{key: 'value', num: 42}` |

### Operators

#### Comparison Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `==` | Equals | `input.status == 'active'` |
| `!=` | Not equals | `input.type != 'draft'` |
| `>` | Greater than | `input.count > 10` |
| `<` | Less than | `input.priority < 5` |
| `>=` | Greater or equal | `input.score >= 80` |
| `<=` | Less or equal | `input.rank <= 3` |

#### Logical Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `&&` | Logical AND | `input.active && input.verified` |
| `\|\|` | Logical OR | `input.admin \|\| input.moderator` |
| `!` | Logical NOT | `!input.disabled` |

### Field Access

```rel
// Property access
input.value
input.user.name
context.settings.theme

// Index access
input.tags[0]
input.items[2]
data["key"]
```

## Null-Safe Chaining

REL uses **implicit null-safe chaining** (similar to JavaScript's `?.` operator but without the `?`). When accessing a property or calling a method on a null value, the expression returns `null` instead of throwing an error.

```rel
// If input.meta is null or missing, the entire expression returns null
input.meta.settings.theme

// Method calls on null also return null
input.missing.field.contains('test')  // Returns null, not an error

// This enables safe boolean checks
input.meta && input.meta.published  // Works like JavaScript
```

## Methods

REL uses method-chaining syntax for all operations. Methods are called on values using dot notation.

### Universal Methods

These methods work on strings, arrays, and objects.

| Method | Syntax | Description |
|--------|--------|-------------|
| `length()` | `x.length()` | Get length (string chars, array elements, or object keys) |
| `isEmpty()` | `x.isEmpty()` | Check if empty string, empty array, empty object, or null |
| `isNotEmpty()` | `x.isNotEmpty()` | Check if not empty and not null |

```rel
input.name.length() > 10
input.tags.isEmpty()
input.data.isNotEmpty()
```

### String Methods

| Method | Syntax | Description |
|--------|--------|-------------|
| `contains(substr)` | `str.contains('test')` | Check if string contains substring |
| `startsWith(prefix)` | `str.startsWith('/api')` | Check if string starts with prefix |
| `endsWith(suffix)` | `str.endsWith('.txt')` | Check if string ends with suffix |
| `toLowerCase()` | `str.toLowerCase()` | Convert to lowercase |
| `toUpperCase()` | `str.toUpperCase()` | Convert to uppercase |
| `trim()` | `str.trim()` | Remove leading/trailing whitespace |
| `substring(start, end?)` | `str.substring(0, 5)` | Extract substring (end is optional) |

```rel
input.email.contains('@')
input.path.startsWith('/content')
input.filename.endsWith('.json')
input.name.toLowerCase().contains('admin')
input.code.toUpperCase() == 'ABC'
input.text.trim().isNotEmpty()
input.id.substring(0, 3) == 'PRE'
```

### Array Methods

| Method | Syntax | Description |
|--------|--------|-------------|
| `contains(element)` | `arr.contains('admin')` | Check if array contains element |
| `first()` | `arr.first()` | Get first element (null if empty) |
| `last()` | `arr.last()` | Get last element (null if empty) |
| `indexOf(element)` | `arr.indexOf('x')` | Get index of element (-1 if not found) |
| `join(separator?)` | `arr.join(', ')` | Join elements into string |

```rel
input.roles.contains('admin')
input.tags.first() == 'important'
input.history.last()
input.items.indexOf('target') >= 0
input.names.join(', ')
```

### Path Methods

Path methods are designed for working with hierarchical content paths like `/content/blog/post1`.

| Method | Syntax | Description |
|--------|--------|-------------|
| `parent(n?)` | `path.parent()` | Get parent path (n levels up, default 1) |
| `ancestor(depth)` | `path.ancestor(2)` | Get ancestor at absolute depth from root |
| `ancestorOf(path)` | `path.ancestorOf('/a/b')` | Check if this path is ancestor of given path |
| `descendantOf(path)` | `path.descendantOf('/a')` | Check if this path is descendant of given path |
| `childOf(path)` | `path.childOf('/a')` | Check if this path is direct child of given path |
| `depth()` | `path.depth()` | Get hierarchy depth |

```rel
// Given path = '/content/blog/post1'

input.node.path.parent()                    // '/content/blog'
input.node.path.parent(2)                   // '/content'
input.node.path.ancestor(1)                 // '/content'
input.node.path.descendantOf('/content')    // true
input.node.path.childOf('/content/blog')    // true
input.node.path.ancestorOf('/content/blog/post1/comments')  // true
input.node.path.depth()                     // 3

// Combined with string methods
input.node.path.parent().endsWith('/blog')  // true
```

## Graph Relationship Checks (RELATES)

REL supports graph-based permission conditions using the `RELATES` keyword. This allows you to check if nodes have relationships through the graph, enabling use cases like "show posts only from friends" or "friends of friends".

### Basic Syntax

```
source RELATES target VIA 'RELATION_TYPE'
source RELATES target VIA 'RELATION_TYPE' DEPTH min..max
source RELATES target VIA ['TYPE1', 'TYPE2'] DIRECTION OUTGOING
```

### Components

| Component | Description | Required |
|-----------|-------------|----------|
| `source` | Expression resolving to source node ID | Yes |
| `target` | Expression resolving to target node ID | Yes |
| `VIA` | Relationship type(s) to traverse | Yes |
| `DEPTH` | Path length range (default: 1) | No |
| `DIRECTION` | Traversal direction (default: ANY) | No |

**Important:** RELATES operates on **node IDs**, not paths. The graph relationship index stores relationships between node IDs. Use properties that contain node IDs:

| Property | Type | Example Value |
|----------|------|---------------|
| `node.created_by` | Node ID | `"user123"` |
| `node.owner_id` | Node ID | `"user456"` |
| `auth.local_user_id` | Node ID | `"user789"` |
| `node.id` | Node ID | `"node-abc"` |

Path-based properties like `node.path` or `auth.home` contain paths (e.g., `/users/alice`) and cannot be used directly with RELATES because the relationship index stores node IDs.

### DEPTH Options

| Syntax | Description |
|--------|-------------|
| `DEPTH 1` | Exactly 1 hop (direct relationship) |
| `DEPTH 2` | Exactly 2 hops |
| `DEPTH 1..2` | Between 1 and 2 hops (friends or friends-of-friends) |
| `DEPTH 1..3` | Between 1 and 3 hops |
| (omitted) | Default is 1 (direct relationship) |

### DIRECTION Options

| Value | Description |
|-------|-------------|
| `ANY` | Either direction (default) |
| `OUTGOING` | source → target |
| `INCOMING` | target → source |

### Examples

```rel
// Direct friendship check (1 hop)
node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'

// Friends or friends-of-friends (1-2 hops)
node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 1..2

// Multiple relationship types
node.created_by RELATES auth.local_user_id VIA ['FRIENDS_WITH', 'FOLLOWS']

// Directed relationship (outgoing only)
node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DIRECTION OUTGOING

// Full syntax with all options
node.author RELATES auth.local_user_id VIA ['FRIENDS_WITH', 'COLLEAGUE_OF'] DEPTH 1..3 DIRECTION ANY
```

### Use in Permission Rules

```yaml
permissions:
  - path: "posts.**"
    operations: [read]
    condition: |
      node.visibility == 'public' ||
      node.created_by == auth.local_user_id ||
      node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 1..2

  - path: "messages.**"
    operations: [read]
    condition: |
      node.recipient == auth.local_user_id ||
      node.sender RELATES auth.local_user_id VIA 'FRIENDS_WITH'
```

### Combining with Other Conditions

RELATES expressions can be combined with other REL expressions using logical operators:

```rel
// Owner OR friend can see
node.created_by == auth.local_user_id ||
node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'

// Must be friend AND have editor role
node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' &&
auth.roles.contains('editor')

// Public posts OR friends' posts OR own posts
node.visibility == 'public' ||
node.created_by == auth.local_user_id ||
node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 1..2
```

### Performance Notes

- RELATES uses BFS (Breadth-First Search) with early termination
- Cross-workspace relationships are supported via the global relation index
- For high-volume queries, consider limiting depth to reduce traversal cost
- Results are evaluated at query time (live query)

## Method Chaining

Methods can be chained together for complex operations:

```rel
// Chain string methods
input.name.trim().toLowerCase().contains('admin')

// Chain with comparisons
input.text.trim().length() > 0

// Chain path methods with string methods
input.node.path.parent(2).startsWith('/content')
```

## Boolean Existence Checks

REL supports JavaScript-style boolean existence checks. Values are evaluated as truthy/falsy:

- **Falsy values**: `null`, `false`, `0`, `''` (empty string), `[]` (empty array), `{}` (empty object)
- **Truthy values**: Everything else

```rel
// Check if property exists and has a value
input.meta && input.meta.published

// Short-circuit evaluation
input.config && input.config.enabled && input.config.value > 10

// Using isNotEmpty for explicit checks
input.name.isNotEmpty()
```

## Operator Precedence

From lowest to highest precedence:

1. `||` (OR)
2. `&&` (AND)
3. `==`, `!=`, `<`, `>`, `<=`, `>=` (comparison)
4. `!` (NOT - unary)
5. `-` (negation - unary)
6. `.` and `[...]` (property/index access, method calls)
7. Atoms (literals, variables, parentheses)

Use parentheses to override precedence:

```rel
// Without parentheses: a || (b && c) due to precedence
a || b && c

// With parentheses: (a || b) && c
(a || b) && c
```

## Complete Examples

### Trigger Conditions

```rel
// Trigger on new blog posts
input.node.path.descendantOf('/content/blog') && input.type == 'create'

// Trigger on high-priority items from specific users
input.priority >= 8 && input.author.role.contains('editor')

// Trigger on published content updates
input.meta.published == true && input.type == 'update'
```

### Flow Conditions

```rel
// Route based on content type
input.contentType.startsWith('image/')

// Check user permissions
input.user.roles.contains('admin') || input.user.roles.contains('moderator')

// Validate input data
input.email.contains('@') && input.name.trim().isNotEmpty()
```

### Complex Conditions

```rel
// Multi-condition check with path hierarchy
(input.node.path.descendantOf('/content/blog') ||
 input.node.path.descendantOf('/content/news')) &&
input.meta.status == 'published' &&
input.meta.author.isNotEmpty()

// Nested property checks with null safety
input.config &&
input.config.features &&
input.config.features.contains('advanced') &&
input.config.settings.maxItems > 100
```

## Error Handling

REL provides clear error messages for common issues:

| Error | Description |
|-------|-------------|
| `UndefinedVariable` | Referenced variable doesn't exist in context |
| `UnknownMethod` | Called method doesn't exist |
| `TypeError` | Type mismatch in operation |
| `IndexOutOfBounds` | Array index out of range |
| `DivisionByZero` | Division by zero attempted |

## Usage in RaisinDB

### In Triggers

```yaml
triggers:
  - name: notify-on-publish
    condition: "input.meta.published == true && input.type == 'update'"
    action: send-notification
```

### In Flow Conditions

```yaml
flows:
  - name: content-router
    steps:
      - condition: "input.node.path.startsWith('/api')"
        action: api-handler
      - condition: "input.node.path.startsWith('/content')"
        action: content-handler
```

### In JavaScript Functions

```javascript
// REL expressions are evaluated in the function context
const condition = "input.value > threshold && input.enabled"
```

## Best Practices

1. **Use null-safe access**: REL handles null gracefully, but be explicit with `&&` checks for clarity
2. **Prefer methods over functions**: Use `str.contains('x')` not `contains(str, 'x')`
3. **Chain methods for readability**: `input.name.trim().toLowerCase()` is cleaner than nested operations
4. **Use path methods for hierarchies**: They're optimized for `/` separated paths
5. **Keep conditions simple**: Break complex logic into multiple conditions when possible
