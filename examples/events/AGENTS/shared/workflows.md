# Development Workflows

## Create a Node Type

1. Design the schema -- decide on properties, types, and indexes
2. Create `nodetypes/{namespace}:{Name}.yaml`
3. Optionally create an archetype in `archetypes/` for editor support
4. Add the NodeType name to `manifest.yaml` under `provides.nodetypes`
5. Add to workspace `allowed_node_types` if content should be editable there
6. Validate: `raisindb package create --check .`

See `AGENTS/tasks/create-node-type.md` for detailed steps.

## Add a Trigger

1. Create a directory under `content/functions/triggers/on-{event}/`
2. Add `.node.yaml` with trigger configuration (event type, filter)
3. Write `handler.ts` with the event handler logic
4. Add the trigger path to `manifest.yaml` under `provides.triggers`
5. Validate: `raisindb package create --check .`

## Create a Library Function

1. Create a directory under `content/functions/lib/{namespace}/{fn-name}/`
2. Add `.node.yaml` with function metadata (name, parameters)
3. Write `handler.ts` implementing the function logic
4. Add the function path to `manifest.yaml` under `provides.functions`
5. Validate: `raisindb package create --check .`

## Validate Package

Always validate before uploading:

```bash
raisindb package create --check .
```

This checks:
- manifest.yaml is valid and complete
- All referenced NodeTypes, Archetypes, ElementTypes exist
- Workspace definitions are consistent
- Function and trigger configurations are valid
- Content nodes conform to their NodeType schemas

## Build and Upload

```bash
# Build the .rap archive
raisindb package create .

# Upload to server
raisindb package upload {packageName}-0.1.0.rap
```

## Live Development with Sync

During development, use sync for instant feedback:

```bash
raisindb package sync .
```

This watches for file changes and pushes updates to the server without
rebuilding the full package. Faster iteration than build + upload.
