---
sidebar_position: 2
---

# Property Types Reference

This page reflects `PropertyType` and `PropertyValue` from `raisin-models`.

## PropertyType (schema)

When defining NodeType properties, use these types in `property_type`:

- String
- Number
- Boolean
- Date
- URL
- Reference
- NodeType (reference to a NodeType definition)
- Block (single content block)
- BlockContainer (collection of blocks)
- Resource (file/binary metadata)
- Array (of nested schemas)
- Object (inline map of nested schemas)

Schema field:

```yaml
properties:
  - name: title
    type: String
    required: true
  - name: author
    type: Reference
  - name: gallery
    type: Array
    items:
      type: Resource
```

Additional schema attributes:
- required: boolean
- unique: boolean
- default: PropertyValue (see below)
- constraints: map[string]PropertyValue (custom rules)
- structure: map[string]PropertyValueSchema (for Object)
- items: PropertyValueSchema (for Array)
- is_translatable: boolean
- allow_additional_properties: boolean

## PropertyValue (runtime values)

Values sent/stored at runtime can be one of:

- String: "text"
- Number: 3.14
- Boolean: true
- Date: ISO 8601 timestamp (e.g., 2024-07-01T12:34:56Z)
- URL: "https://example.com"
- Reference:
  ```json
  {
    "raisin:ref": "<node-id>",
    "raisin:workspace": "main",
    "raisin:path": "/articles/welcome"
  }
  ```
- Resource:
  ```json
  {
    "uuid": "res-uuid",
    "name": "file.png",
    "size": 12345,
    "mime_type": "image/png",
    "url": "/assets/file.png",
    "metadata": {},
    "is_loaded": true,
    "is_external": false,
    "created_at": "2024-07-01T12:34:56Z",
    "updated_at": "2024-07-01T12:34:56Z"
  }
  ```
- Block:
  ```json
  {"uuid": "blk-1", "block_type": "paragraph", "content": {"text": "Hello"}}
  ```
- BlockContainer:
  ```json
  {"uuid": "cnt-1", "items": [{"uuid": "blk-1", "block_type": "paragraph", "content": {"text": "Hello"}}]}
  ```
- Array: [PropertyValue, ...]
 - Array: `[PropertyValue, ...]`
 - Object: `{"key": PropertyValue, ...}`

## Examples

### Article NodeType

```yaml
name: blog:Article
properties:
  - name: title
    type: String
    required: true
  - name: content
    type: BlockContainer
  - name: author
    type: Reference
  - name: cover
    type: Resource
  - name: tags
    type: Array
    items:
      type: String
```
