# Stewardship Functions

This document describes the QuickJS functions available for querying and managing stewardship relationships in RaisinDB.

## Overview

The stewardship system provides three core functions:

1. **get-stewards** - Get all stewards for a given ward
2. **get-wards** - Get all wards for a given steward
3. **is-steward-of** - Check if one user is a steward of another

All functions are implemented in Python and run in the function runtime with access to the RaisinDB API.

### Function Locations

Functions are defined in `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/lib/stewardship/`:

- `get-stewards/index.py`
- `get-wards/index.py`
- `is-steward-of/index.py`

---

## get-stewards

Gets all stewards for a given ward user.

### Description

Returns users who have stewardship relationships (PARENT_OF, GUARDIAN_OF) with the specified ward. For PARENT_OF relationships, only returns parents if the ward is a minor (based on birth_date and minor_age_threshold configuration).

### Input Schema

```typescript
{
  ward_user_path: string;    // Required: Path to ward user (e.g., "/users/internal/alice")
  workspace?: string;        // Optional: Workspace name (default: "raisin:access_control")
}
```

### Output Schema

```typescript
{
  success: boolean;
  stewards?: Array<{
    user_id: string;         // UUID of steward user
    user_path: string;       // Node path of steward
    email?: string;          // Steward email
    display_name?: string;   // Steward display name
    relation_type: string;   // Graph relation type (e.g., "GUARDIAN_OF")
    relation_title: string;  // Human-friendly title (e.g., "Guardian Of")
  }>;
  error?: string;            // Error message if success is false
}
```

### Example Usage

#### Via API

```bash
POST /api/v1/repositories/{repo}/functions/invoke
Content-Type: application/json

{
  "function_path": "/functions/lib/stewardship/get-stewards",
  "input": {
    "ward_user_path": "/users/internal/alice"
  }
}
```

Response:

```json
{
  "success": true,
  "stewards": [
    {
      "user_id": "550e8400-e29b-41d4-a716-446655440000",
      "user_path": "/users/internal/john-smith",
      "email": "john@example.com",
      "display_name": "John Smith",
      "relation_type": "GUARDIAN_OF",
      "relation_title": "Guardian Of"
    },
    {
      "user_id": "660e8400-e29b-41d4-a716-446655440001",
      "user_path": "/users/internal/jane-smith",
      "email": "jane@example.com",
      "display_name": "Jane Smith",
      "relation_type": "PARENT_OF",
      "relation_title": "Parent Of"
    }
  ]
}
```

#### From JavaScript Function

```javascript
async function myFunction(input) {
    const result = await raisin.functions.invoke(
        "/functions/lib/stewardship/get-stewards",
        {
            ward_user_path: "/users/internal/alice",
            workspace: "raisin:access_control"
        }
    );

    if (result.success) {
        console.log(`Found ${result.stewards.length} stewards for ward`);
        for (const steward of result.stewards) {
            console.log(`${steward.display_name} (${steward.relation_title})`);
        }
    } else {
        console.error(`Error: ${result.error}`);
    }

    return result;
}
```

### Implementation Details

The function:

1. Retrieves the ward user node
2. Loads stewardship configuration from `/config/stewardship`
3. Calculates ward's age if birth_date is present
4. Queries incoming graph relations using SQL NEIGHBORS function
5. Filters PARENT_OF relations based on minor status
6. Enriches results with user details and relation type metadata

See source: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/lib/stewardship/get-stewards/index.py`

---

## get-wards

Gets all wards for a given steward user.

### Description

Returns users for whom the steward has stewardship relationships (PARENT_OF, GUARDIAN_OF). For PARENT_OF relationships, only includes wards who are minors (based on birth_date and minor_age_threshold configuration).

### Input Schema

```typescript
{
  steward_user_path: string;  // Required: Path to steward user (e.g., "/users/internal/bob")
  workspace?: string;         // Optional: Workspace name (default: "raisin:access_control")
}
```

### Output Schema

```typescript
{
  success: boolean;
  wards?: Array<{
    user_id: string;         // UUID of ward user
    user_path: string;       // Node path of ward
    email?: string;          // Ward email
    display_name?: string;   // Ward display name
    relation_type: string;   // Graph relation type (e.g., "PARENT_OF")
    relation_title: string;  // Human-friendly title (e.g., "Parent Of")
    is_minor: boolean;       // Whether ward is currently a minor
  }>;
  error?: string;            // Error message if success is false
}
```

### Example Usage

#### Via API

```bash
POST /api/v1/repositories/{repo}/functions/invoke
Content-Type: application/json

{
  "function_path": "/functions/lib/stewardship/get-wards",
  "input": {
    "steward_user_path": "/users/internal/john-smith"
  }
}
```

Response:

```json
{
  "success": true,
  "wards": [
    {
      "user_id": "770e8400-e29b-41d4-a716-446655440002",
      "user_path": "/users/internal/alice-smith",
      "email": "alice@example.com",
      "display_name": "Alice Smith",
      "relation_type": "PARENT_OF",
      "relation_title": "Parent Of",
      "is_minor": true
    },
    {
      "user_id": "880e8400-e29b-41d4-a716-446655440003",
      "user_path": "/users/internal/bob-jones",
      "email": "bob@example.com",
      "display_name": "Bob Jones",
      "relation_type": "GUARDIAN_OF",
      "relation_title": "Guardian Of",
      "is_minor": true
    }
  ]
}
```

#### From JavaScript Function

```javascript
async function listMyWards(input) {
    const stewardPath = input.user_path;

    const result = await raisin.functions.invoke(
        "/functions/lib/stewardship/get-wards",
        {
            steward_user_path: stewardPath,
            workspace: "raisin:access_control"
        }
    );

    if (result.success) {
        const minors = result.wards.filter(w => w.is_minor);
        const adults = result.wards.filter(w => !w.is_minor);

        console.log(`Total wards: ${result.wards.length}`);
        console.log(`Minors: ${minors.length}`);
        console.log(`Adults: ${adults.length}`);
    }

    return result;
}
```

### Implementation Details

The function:

1. Retrieves the steward user node
2. Loads stewardship configuration from `/config/stewardship`
3. Queries outgoing graph relations using SQL NEIGHBORS function
4. For each ward, calculates age to determine minor status
5. Filters PARENT_OF relations to only include minors
6. Enriches results with user details and relation type metadata

See source: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/lib/stewardship/get-wards/index.py`

---

## is-steward-of

Checks if user A is a steward of user B.

### Description

Returns true if there is a stewardship-implying relationship (PARENT_OF, GUARDIAN_OF) from A to B. For PARENT_OF relationships, considers the minor status of B based on birth_date and minor_age_threshold configuration.

### Input Schema

```typescript
{
  steward_user_path: string;  // Required: Path to potential steward (e.g., "/users/internal/bob")
  ward_user_path: string;     // Required: Path to potential ward (e.g., "/users/internal/alice")
  workspace?: string;         // Optional: Workspace name (default: "raisin:access_control")
}
```

### Output Schema

```typescript
{
  success: boolean;
  is_steward?: boolean;       // True if stewardship relationship exists
  relation_type?: string;     // Graph relation type if is_steward is true
  relation_title?: string;    // Human-friendly title if is_steward is true
  error?: string;             // Error message if success is false
}
```

### Example Usage

#### Via API

```bash
POST /api/v1/repositories/{repo}/functions/invoke
Content-Type: application/json

{
  "function_path": "/functions/lib/stewardship/is-steward-of",
  "input": {
    "steward_user_path": "/users/internal/john-smith",
    "ward_user_path": "/users/internal/alice-smith"
  }
}
```

Response (stewardship exists):

```json
{
  "success": true,
  "is_steward": true,
  "relation_type": "PARENT_OF",
  "relation_title": "Parent Of"
}
```

Response (no stewardship):

```json
{
  "success": true,
  "is_steward": false
}
```

#### From JavaScript Function

```javascript
async function checkAccess(input) {
    const { current_user_path, target_user_path } = input;

    // Check if current user is a steward of target user
    const stewardCheck = await raisin.functions.invoke(
        "/functions/lib/stewardship/is-steward-of",
        {
            steward_user_path: current_user_path,
            ward_user_path: target_user_path,
            workspace: "raisin:access_control"
        }
    );

    if (stewardCheck.success && stewardCheck.is_steward) {
        console.log(`User has ${stewardCheck.relation_title} relationship`);
        return { access_granted: true, reason: "stewardship" };
    }

    return { access_granted: false };
}
```

#### In Permission Conditions

```yaml
# RLS permission rule
permissions:
  - resource: "user_data"
    action: "read"
    condition: |
      // Use is-steward-of to check relationship
      const result = await raisin.functions.invoke(
          "/functions/lib/stewardship/is-steward-of",
          {
              steward_user_path: $auth.user_path,
              ward_user_path: input.node.path
          }
      );
      return result.success && result.is_steward;
```

### Implementation Details

The function:

1. Retrieves both steward and ward user nodes
2. Loads stewardship configuration from `/config/stewardship`
3. Calculates ward's age to determine minor status
4. Queries for a direct graph relation from steward to ward
5. For PARENT_OF relations, only returns true if ward is a minor
6. Returns relationship metadata if stewardship exists

See source: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/lib/stewardship/is-steward-of/index.py`

---

## Function Invocation Methods

### 1. REST API

The most common method for external applications:

```bash
POST /api/v1/repositories/{repo}/functions/invoke
Authorization: Bearer {token}
Content-Type: application/json

{
  "function_path": "/functions/lib/stewardship/get-stewards",
  "input": {
    "ward_user_path": "/users/internal/alice"
  }
}
```

### 2. From Other Functions

Functions can call other functions using `raisin.functions.invoke()`:

```javascript
async function myFunction(input) {
    const result = await raisin.functions.invoke(
        "/functions/lib/stewardship/get-stewards",
        { ward_user_path: "/users/internal/alice" }
    );

    return result;
}
```

### 3. From SQL via Function Calls

```sql
SELECT raisin_function(
    'raisin:access_control',
    '/functions/lib/stewardship/get-stewards',
    '{"ward_user_path": "/users/internal/alice"}'::jsonb
) as result;
```

### 4. GraphQL (if enabled)

```graphql
mutation InvokeFunction {
  invokeFunction(
    functionPath: "/functions/lib/stewardship/get-stewards"
    input: {
      ward_user_path: "/users/internal/alice"
    }
  ) {
    success
    stewards {
      user_id
      display_name
      relation_title
    }
  }
}
```

---

## RaisinDB API Available in Functions

Functions have access to the following RaisinDB APIs:

### Node Operations

```javascript
// Get node by path
const node = await raisin.nodes.get(workspace, path);

// Get node by ID
const node = await raisin.nodes.getById(workspace, nodeId);

// Create node
const newNode = await raisin.nodes.create(workspace, {
    path: "/path/to/node",
    node_type: "raisin:SomeType",
    properties: { ... }
});

// Update node
await raisin.nodes.update(workspace, path, {
    properties: { status: "active" }
});

// Delete node
await raisin.nodes.delete(workspace, path);
```

### SQL Queries

```javascript
// Execute SQL query
const results = await raisin.sql.query(
    `SELECT * FROM nodes WHERE node_type = $1`,
    ["raisin:User"]
);

// Use NEIGHBORS function for graph traversal
const relations = await raisin.sql.query(
    `SELECT e.dst_id, e.edge_label
     FROM NEIGHBORS($1, 'OUT', NULL) AS e
     WHERE e.edge_label = 'GUARDIAN_OF'`,
    [userId]
);
```

### Graph Operations

```javascript
// Create graph relation
await raisin.graph.createRelation(
    workspace,
    sourceNodeId,
    targetNodeId,
    "GUARDIAN_OF"
);

// Delete graph relation
await raisin.graph.deleteRelation(
    workspace,
    sourceNodeId,
    targetNodeId,
    "GUARDIAN_OF"
);
```

### Logging

```javascript
console.log("Info message");
console.warn("Warning message");
console.error("Error message");
```

---

## Creating Custom Functions

### 1. Create Function File

Create `/functions/custom/my-function/index.js`:

```javascript
/**
 * My custom stewardship function
 */
async function myCustomFunction(input) {
    const { user_path } = input;

    try {
        // Your logic here
        const result = await raisin.nodes.get("raisin:access_control", user_path);

        return {
            success: true,
            data: result
        };
    } catch (error) {
        return {
            success: false,
            error: error.message
        };
    }
}

module.exports = { myCustomFunction };
```

### 2. Create Function Definition

Create `/functions/custom/my-function/.node.yaml`:

```yaml
node_type: raisin:Function
properties:
  title: My Custom Function
  description: Does something custom
  language: JavaScript
  entry_file: index.js:myCustomFunction
  execution_mode: Async
  enabled: true
  resource_limits:
    timeout_ms: 5000
    max_memory_bytes: 33554432
  input_schema:
    type: object
    required:
      - user_path
    properties:
      user_path:
        type: string
        description: User path
  output_schema:
    type: object
    properties:
      success:
        type: boolean
      data:
        type: object
      error:
        type: string
```

### 3. Register in Package Manifest

Add to package `manifest.yaml`:

```yaml
functions:
  - path: /functions/custom/my-function
    handler: myCustomFunction
```

### 4. Test Function

```bash
POST /api/v1/repositories/{repo}/functions/invoke
Content-Type: application/json

{
  "function_path": "/functions/custom/my-function",
  "input": {
    "user_path": "/users/internal/alice"
  }
}
```

---

## Best Practices

1. **Always Return Structured Responses**: Use `{ success, data/error }` pattern
2. **Validate Input**: Check required parameters before processing
3. **Handle Errors Gracefully**: Use try-catch and return error messages
4. **Log for Debugging**: Use `console.log()` extensively
5. **Keep Functions Focused**: One function should do one thing well
6. **Document Input/Output**: Use clear JSDoc comments
7. **Test Thoroughly**: Test with various inputs and edge cases
8. **Optimize Queries**: Use SQL efficiently, avoid N+1 queries
9. **Respect Timeouts**: Keep functions under 5 seconds
10. **Use Async/Await**: All RaisinDB APIs are async

---

## Performance Considerations

### SQL vs Node API

For bulk operations, SQL is faster:

```javascript
// Slower: Multiple node API calls
for (const wardId of wardIds) {
    const ward = await raisin.nodes.getById(workspace, wardId);
    wards.push(ward);
}

// Faster: Single SQL query
const wards = await raisin.sql.query(
    `SELECT * FROM nodes WHERE id = ANY($1)`,
    [wardIds]
);
```

### Caching Configuration

Cache frequently accessed configuration:

```javascript
let cachedConfig = null;

async function getConfig() {
    if (!cachedConfig) {
        cachedConfig = await raisin.nodes.get(
            "raisin:access_control",
            "/config/stewardship"
        );
    }
    return cachedConfig;
}
```

### Batch Processing

Process items in batches for large datasets:

```javascript
const BATCH_SIZE = 100;
for (let i = 0; i < items.length; i += BATCH_SIZE) {
    const batch = items.slice(i, i + BATCH_SIZE);
    await processBatch(batch);
}
```

---

## See Also

- [Node Types](./node-types.md) - Stewardship node type definitions
- [Triggers](./triggers.md) - How triggers invoke functions
- [API Reference](./api-reference.md) - REST API endpoints
- [Extending Stewardship](./extending.md) - Adding custom functionality
