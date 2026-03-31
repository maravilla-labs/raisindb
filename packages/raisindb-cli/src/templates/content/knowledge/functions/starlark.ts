export function functionsStarlarkKnowledge(): string {
  return `# Starlark Functions

Starlark functions use a Python-like syntax with synchronous \`raisin.*\` API calls (the runtime handles async internally).

## Handler Pattern

\`\`\`python
def handler(input):
    workspace = input.get("workspace", "raisin:access_control")

    # Your logic here
    return {"success": True}
\`\`\`

- File extension is \`.py\`, language is \`Starlark\` (or \`python\`)
- The function name must match \`entry_file\`: \`index.py:handler\`
- Input is a dict; access fields with \`input.get("key")\` or \`input["key"]\`
- Return a dict matching \`output_schema\`

## .node.yaml

\`\`\`yaml
node_type: raisin:Function
properties:
  name: my-function
  title: My Function
  description: Handles something
  language: Starlark
  entry_file: index.py:handler
  execution_mode: Async
  enabled: true
  resource_limits:
    timeout_ms: 5000
    max_memory_bytes: 33554432
  input_schema:
    type: object
    properties:
      workspace:
        type: string
  output_schema:
    type: object
    properties:
      success:
        type: boolean
\`\`\`

## Key Differences from JavaScript

| Aspect | JavaScript | Starlark |
|--------|-----------|----------|
| File extension | \`.js\` | \`.py\` |
| Async | \`async/await\` | Synchronous (runtime wraps) |
| Error handling | \`try/catch\` | \`fail("message")\` |
| Logging | \`console.log()\` | \`log.info()\` / \`print()\` |
| Transactions | Supported | Not available |
| Null check | \`if (!val)\` | \`if not val:\` |
| Booleans | \`true\` / \`false\` | \`True\` / \`False\` |
| None | \`null\` / \`undefined\` | \`None\` |

## Common Patterns

### Query and process nodes

\`\`\`python
def handler(input):
    workspace = input.get("workspace", "raisin:access_control")
    user_path = input.get("user_path")

    if not user_path:
        fail("user_path is required")

    user = raisin.nodes.get(workspace, user_path)
    if not user:
        fail("User not found at: " + user_path)

    result = raisin.sql.query("""
        SELECT id, path, properties
        FROM 'raisin:access_control'
        WHERE node_type = 'raisin:User'
          AND properties->>'email'::String = $1
    """, [user.get("properties", {}).get("email")])

    # Handle result format
    if type(result) == "list":
        rows = result
    else:
        rows = result.get("rows", []) if result else []

    return {"success": True, "count": len(rows)}
\`\`\`

### Create nodes

\`\`\`python
def handler(input):
    workspace = input.get("workspace", "raisin:access_control")
    parent_path = input.get("parent_path")
    title = input.get("title", "Untitled")

    slug = "item-" + str(raisin.date.timestamp())

    raisin.nodes.create(workspace, parent_path, {
        "name": slug,
        "slug": slug,
        "node_type": "raisin:Message",
        "properties": {
            "title": title,
            "status": "pending",
            "created_at": raisin.date.now()
        }
    })

    return {"success": True, "path": parent_path + "/" + slug}
\`\`\`

### Read and update properties

\`\`\`python
def handler(input):
    workspace = input.get("workspace", "raisin:access_control")
    path = input.get("path")

    node = raisin.nodes.get(workspace, path)
    if not node:
        fail("Node not found: " + path)

    props = node.get("properties", {})
    props["status"] = "processed"
    props["processed_at"] = raisin.date.now()

    raisin.nodes.update(workspace, path, {"properties": props})

    log.info("Processed node: " + path)
    return {"success": True}
\`\`\`

### Error handling

\`\`\`python
def handler(input):
    required_field = input.get("required_field")
    if not required_field:
        fail("required_field is required")

    # fail() immediately stops execution and returns an error
    # For recoverable errors, return an error dict instead:
    node = raisin.nodes.get("myworkspace", "/some/path")
    if not node:
        return {"success": False, "error": "Node not found"}

    return {"success": True}
\`\`\`

## Logging

\`\`\`python
log.debug("Detailed info for debugging")
log.info("General information")
log.warn("Something might be wrong")
log.error("Something went wrong")
print("Also works for basic output")
\`\`\`

## Date API

\`\`\`python
now_iso = raisin.date.now()          # "2025-01-15T10:30:00Z"
ts = raisin.date.timestamp()          # 1736937000 (Unix seconds)
formatted = raisin.date.format(ts)    # ISO string from timestamp
\`\`\`
`;
}
