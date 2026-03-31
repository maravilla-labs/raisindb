---
sidebar_position: 1
---

# HTTP & WebSocket Errors

Both REST and WebSocket APIs return the same JSON/MessagePack structure defined in `crates/raisin-transport-http/src/error.rs`.

```json title="Error payload"
{
  "code": "NODE_NOT_FOUND",
  "message": "Node not found at path: /home",
  "details": "Optional low-level context",
  "field": "Optional field name for validation errors",
  "timestamp": "2025-01-01T00:00:00Z"
}
```

- `code` – machine-readable keyword (see tables below).
- `message` – concise, human-readable explanation.
- `details` – stack traces or backend messages (omitted unless needed).
- `field` – provided on validation errors when the offending field is known.
- `timestamp` – generated per request to help with tracing.

## Status Codes and Error Families

### 400 Bad Request

| Code | Use Case |
|------|----------|
| `INVALID_NODE_TYPE` | The supplied NodeType name is unknown |
| `INVALID_BRANCH_NAME` | Branch names fail validation |
| `INVALID_REVISION_NUMBER` | Revision path segments were not positive integers |
| `NODE_ALREADY_PUBLISHED` | Attempting to edit a published node |
| `VALIDATION_FAILED` | Generic schema validation error |
| `MISSING_REQUIRED_FIELD` | Payload omitted a required field |
| `INVALID_JSON` | Body was not valid JSON |
| `PAYLOAD_TOO_LARGE` | Payload exceeded the configured maximum |
| `ENCODING_ERROR` | Serialization/encoding issue bubbled up from storage |

### 401 / 403

| Code | Meaning |
|------|---------|
| `UNAUTHORIZED` | Auth token missing or invalid |
| `FORBIDDEN` | Authenticated user lacks permission |

### 404 Not Found

| Code | Meaning |
|------|---------|
| `NODE_NOT_FOUND` | Node ID/path not found |
| `BRANCH_NOT_FOUND` | Branch missing |
| `TAG_NOT_FOUND` | Tag missing |
| `REPOSITORY_NOT_FOUND` | Repo missing |
| `WORKSPACE_NOT_FOUND` | Workspace missing |
| `NODE_TYPE_NOT_FOUND` | NodeType missing |
| `ARCHETYPE_NOT_FOUND` | Archetype missing |
| `ELEMENT_TYPE_NOT_FOUND` | ElementType missing |
| `REVISION_NOT_FOUND` | Revision (HLC) missing |
| `NOT_FOUND` | Generic fallback |

### 405 Method Not Allowed

| Code | Meaning |
|------|---------|
| `READ_ONLY_REVISION` | Tried to mutate `/rev/{revision}` routes |

### 409 Conflict

| Code | Meaning |
|------|---------|
| `BRANCH_ALREADY_EXISTS` | Branch duplication |
| `TAG_ALREADY_EXISTS` | Tag duplication |
| `REPOSITORY_ALREADY_EXISTS` | Repo duplication |
| `NODE_ALREADY_EXISTS` | Node path already taken |
| `WORKSPACE_ALREADY_EXISTS` | Workspace duplication |

### 422

`VALIDATION_FAILED` doubles as the 422-equivalent for deep schema checks. The HTTP layer currently returns 400 plus `field` metadata; adjust if your client expects 422 specifically.

### 500 Internal Errors

| Code | Meaning |
|------|---------|
| `INTERNAL_SERVER_ERROR` | Catch-all error |
| `STORAGE_ERROR` | RocksDB/storage failure |
| `SERIALIZATION_ERROR` | serde/json/msgpack failure |

## Mapping `raisin_error::Error` to API Errors

`impl From<raisin_error::Error> for ApiError` converts storage/service errors automatically:

- `NotFound` → entity-specific `*_NOT_FOUND`.
- `Validation` → `VALIDATION_FAILED`.
- `Conflict` → entity-specific conflict codes.
- `Backend` → `STORAGE_ERROR`.
- `Unauthorized`/`Forbidden` → HTTP 401/403 codes.
- `Encoding` → `ENCODING_ERROR`.

## WebSocket Errors

WebSocket responses use the same structure via MessagePack. When decoding MessagePack, look for:

```ts
if (response.status === 'error') {
  console.error(response.error.code, response.error.message);
}
```

Keep this reference handy when building SDKs or dashboards that translate RaisinDB errors into UI state.
