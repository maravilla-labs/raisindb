# RaisinDB Triggers Reference

## Overview

Triggers fire server-side functions or flows in response to events. Defined as YAML nodes with `node_type: raisin:Trigger`.

## Trigger Structure

```yaml
node_type: raisin:Trigger
properties:
  title: On Article Published
  name: myapp-on-article-published
  description: Fires when an article is created
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Created
  filters:
    workspaces:
      - myapp
    paths:
      - "**/articles/*"
    node_types:
      - myapp:Article
    property_filters:
      status: published
  priority: 10
  max_retries: 3
  function_path: /lib/myapp/on-article-published
```

## Trigger Types

| Type | Description |
|------|-------------|
| node_event | Fires on node Created, Updated, or Deleted events |
| schedule | Fires on a cron schedule |
| http | Fires on inbound HTTP webhook |

## Event Kinds (node_event)

```yaml
config:
  event_kinds:
    - Created    # Node was created
    - Updated    # Node properties changed
    - Deleted    # Node was removed
```

## Filters

All filters are optional. When multiple are specified, they are ANDed together.

| Filter | Description |
|--------|-------------|
| workspaces | List of workspace names to watch |
| paths | Glob patterns for node paths |
| node_types | List of node type names to match |
| property_filters | Match on property values |

### Path Wildcards

- `*` matches a single path segment
- `**` matches any number of segments

```yaml
filters:
  paths:
    - "articles/*"           # direct children of /articles
    - "**/users/*/outbox/*"  # outbox items at any depth
    - "**"                   # all paths
```

### Property Filters

Simple equality or operators:

```yaml
property_filters:
  status: published                    # exact match
  message_type: direct_message         # exact match
  "file.metadata.storage_key":         # nested key
    $exists: true                      # existence check
  _source:
    $ne: flow                          # not-equal
```

## Targeting a Function vs Flow

Use `function_path` to call a function, or `flow_path` to start a flow:

```yaml
# Call a function
function_path: /lib/myapp/handle-event

# Start a flow
flow_path: /flows/approval-workflow
```

## Priority and Retries

```yaml
priority: 10       # Higher = runs first (default: 10)
max_retries: 3     # Retry on failure (default: 3)
```

## File Location

Triggers live in `package/content/functions/triggers/<name>/.node.yaml` and must be registered in `manifest.yaml` under `provides.triggers`.

## Example: Watch for New Users

```yaml
node_type: raisin:Trigger
properties:
  title: On User Created
  name: myapp-on-user-created
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Created
  filters:
    workspaces:
      - "raisin:access_control"
    node_types:
      - raisin:User
  priority: 10
  max_retries: 3
  function_path: /lib/myapp/welcome-user
```
