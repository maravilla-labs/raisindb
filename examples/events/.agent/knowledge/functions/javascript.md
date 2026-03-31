# JavaScript Functions

JavaScript functions use `async/await` with the `raisin.*` API.

## Handler Pattern

```javascript
async function handler(context) {
  const { event, workspace } = context.flow_input;

  // Your logic here
  return { success: true };
}
```

- The function name must match the `entry_file` in `.node.yaml`: `index.js:handler`
- `context.flow_input` contains the trigger event data
- Return a JSON-serializable object matching `output_schema`

## .node.yaml

```yaml
node_type: raisin:Function
properties:
  name: my-function
  title: My Function
  description: Handles something
  execution_mode: async
  enabled: true
  language: javascript
  entry_file: index.js:handler
  version: 1
  input_schema:
    type: object
    properties:
      flow_input:
        type: object
        properties:
          event:
            type: object
          workspace:
            type: string
  output_schema:
    type: object
    properties:
      success:
        type: boolean
```

## Context Object

When invoked by a trigger, `context.flow_input` contains:

| Field | Description |
|-------|-------------|
| `event.type` | Event kind (Created, Updated, Deleted) |
| `event.node_id` | ID of the affected node |
| `event.node_type` | Node type of the affected node |
| `event.node_path` | Path of the affected node |
| `workspace` | Workspace where the event occurred |

When invoked directly (e.g., as an AI tool), the input fields are passed as top-level properties in the `input` argument.

## Common Patterns

### Query and process nodes

```javascript
async function handler(context) {
  const { event, workspace } = context.flow_input;

  const node = await raisin.nodes.get(workspace, event.node_path);
  if (!node) return { success: false, error: 'Not found' };

  const result = await raisin.sql.query(`
    SELECT id, path, properties
    FROM "raisin:access_control"
    WHERE node_type = 'raisin:User'
      AND properties->>'email'::String = $1
  `, [node.properties.recipient_email]);

  const rows = Array.isArray(result) ? result : (result?.rows || []);
  return { success: true, count: rows.length };
}
```

### Create nodes with transactions

```javascript
async function handler(context) {
  const { workspace } = context.flow_input;

  const tx = await raisin.nodes.beginTransaction();
  try {
    await tx.createDeep('raisin:access_control', '/users/alice/inbox/messages', {
      name: `msg-${Date.now()}`,
      node_type: 'raisin:Message',
      properties: { subject: 'Hello', status: 'delivered' }
    });
    await tx.commit();
    return { success: true };
  } catch (e) {
    await tx.rollback();
    return { success: false, error: e.message };
  }
}
```

### CRUD operations

```javascript
async function handler(input) {
  const { operation, workspace, path, data } = input;

  switch (operation) {
    case 'get': {
      const node = await raisin.nodes.get(workspace, path);
      return { success: !!node, node };
    }
    case 'create': {
      const parentPath = path.split('/').slice(0, -1).join('/');
      await raisin.nodes.create(workspace, parentPath, data);
      return { success: true };
    }
    case 'update': {
      await raisin.nodes.update(workspace, path, { properties: data });
      return { success: true };
    }
    case 'delete': {
      await raisin.nodes.delete(workspace, path);
      return { success: true };
    }
    default:
      return { success: false, error: `Unknown operation: ${operation}` };
  }
}
```

### Error handling

```javascript
async function handler(context) {
  try {
    // ... main logic
    return { success: true };
  } catch (err) {
    console.error('[my-function] Error:', err);
    return { success: false, error: err.message || String(err) };
  }
}
```

## SQL Tips

- Quote workspace names containing colons: `FROM "raisin:access_control"`
- Use `$1`, `$2` for parameterized queries (never string interpolation for values)
- Cast JSON property keys: `properties->>'email'::String = $1`
- Results may be an array or `{ rows: [...] }` -- handle both forms
