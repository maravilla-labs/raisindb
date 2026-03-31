---
sidebar_position: 3
---

# API Examples (curl)

All examples assume the server runs at http://localhost:8080 and `repo=content`, `branch=main`, `ws=site`.

## Create a workspace

```bash
curl -X PUT \
  http://localhost:8080/api/workspaces/content/site \
  -H "Content-Type: application/json" \
  -d '{"name": "site"}' -i
```

## List workspaces

```bash
curl http://localhost:8080/api/workspaces/content
```

## Create a node at root (commit mode)

```bash
curl -X POST \
  http://localhost:8080/api/repository/content/main/head/site/ \
  -H "Content-Type: application/json" \
  -d '{
    "commit": {"message": "Add home page", "actor": "you"},
    "node": {
      "name": "home",
      "node_type": "cms:Page",
      "properties": {"title": "Welcome"}
    }
  }'
```

Response (201):

```json
{
  "node": { /* created node */ },
  "revision": 42,
  "committed": true
}
```

## Get a node by path

```bash
curl "http://localhost:8080/api/repository/content/main/head/site/home"
```

## List children with pagination

```bash
curl "http://localhost:8080/api/repository/content/main/head/site/home/?limit=50"
```

## Query by JSON filter

```bash
curl -X POST \
  http://localhost:8080/api/repository/content/main/head/site/query \
  -H "Content-Type: application/json" \
  -d '{"nodeType": "cms:Page", "parent": "/"}'
```

## Get audit log by path

```bash
curl "http://localhost:8080/api/audit/content/main/site/home"
```

## NodeType management: create

```bash
curl -X POST \
  http://localhost:8080/api/management/content/main/nodetypes \
  -H "Content-Type: application/json" \
  -d '{
    "name": "cms:Page",
    "description": "A simple page",
    "version": 1,
    "properties": [
      {"name": "title", "type": "String", "required": true},
      {"name": "body", "type": "BlockContainer"}
    ],
    "versionable": true,
    "publishable": true
  }'
```

## NodeType: list published

```bash
curl http://localhost:8080/api/management/content/main/nodetypes/published
```

## Branches: list

```bash
curl http://localhost:8080/api/management/repositories/default/content/branches
```
