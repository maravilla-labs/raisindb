---
sidebar_position: 2
---

# Common Request/Response Shapes

These types are defined in code (`crates/raisin-transport-http/src/types.rs` and `crates/raisin-models`).

## Pagination

Response wrapper used by list endpoints and query APIs:

```json
{
  "items": [ /* array of items */ ],
  "page": {
    "total": 123,
    "limit": 50,
    "offset": 0,
    "nextOffset": 50
  }
}
```

Fields:
- page.total number
- page.limit number
- page.offset number
- page.nextOffset number | null

## ErrorBody

```json
{
  "error": "BadRequest",
  "message": "Provide one of: path, parent, nodeType"
}
```

## QueryRequest (JSON filter)

Body for `POST /api/repository/{repo}/{branch}/head/{ws}/query`:

```json
{
  "nodeType": "blog:Article",   // optional
  "parent": "/blog",            // optional
  "path": "/blog/welcome",      // optional, takes precedence if provided
  "limit": 50,                    // optional
  "offset": 0                     // optional
}
```

Rules:
- Provide one of: path, parent, nodeType
- Stable ordering by path then id for pagination

## DSL Query

Body for `POST /api/repository/{repo}/{branch}/head/{ws}/query/dsl` uses `raisin_query::NodeSearchQuery` (JSON). Example:

```json
{
  "filter": {
    "type": "And",
    "items": [
      {"type": "Equals", "field": "nodeType", "value": "blog:Article"},
      {"type": "StartsWith", "field": "path", "value": "/blog/"}
    ]
  },
  "limit": 25,
  "offset": 0
}
```

## RepoQuery (query params)

Used on GET repository routes for directory listing/deep traversal:

```
?level=2&format=array&flatten=false&cursor=BASE64&limit=100&command=download
```

Fields:
- level number (max 10)
- format "array" | "map" (default array)
- flatten boolean
- cursor base64 string (server-provided)
- limit number (default 100, max 1000)
- command "download" (for file downloads)

## CommitInfo and CommitNodeRequest

Commit metadata to create a new revision:

```json
{
  "message": "Create article",
  "actor": "jane.doe"
}
```

Single-node commit body variant:

```json
{
  "message": "Update title",
  "actor": "jane.doe",
  "properties": {"title": "New Title"},
  "node": { /* full node for create operations */ }
}
```

## Node (minimal shape used by repo_post_root)

```json
{
  "id": "nanoid-optional",           // generated if missing
  "name": "welcome",
  "node_type": "blog:Article",
  "path": "/welcome",               // auto-filled as "/{name}" if missing
  "properties": {
    "title": "Hello World",
    "content": {"BlockContainer": {"uuid": "...", "items": []}}
  }
}
```

Notes:
- Server fills `id` and `path` if absent in commit mode
- For direct mode (no commit), body may be the node itself or under `node` key
