# Builtin Packages

Builtin packages are automatically installed when a new repository is created in RaisinDB. They provide foundational functionality like authentication, AI tools, and relationship management.

## Package Structure

Each builtin package directory must contain:

```
package-name/
├── manifest.yaml      # Package metadata and configuration
├── nodetypes/         # Node type definitions (*.yaml)
├── content/           # Content organized by workspace
│   ├── functions/     # Function nodes
│   ├── raisin:access_control/  # Access control nodes
│   └── ...
└── workspaces/        # Workspace definitions (optional)
```

## The `manifest.yaml` File

The manifest defines package metadata and what the package provides:

```yaml
name: my-package
version: 1.0.0
title: My Package Title
description: What this package does
author: Your Name
license: BSL-1.1

# Mark as builtin for auto-install
builtin: true

# What this package provides
provides:
  nodetypes:
    - myns:NodeType1
    - myns:NodeType2
  workspaces:
    - myworkspace
  functions:
    - /functions/lib/myns/my-function
  triggers:
    - /functions/triggers/my-trigger
  content:
    - workspace/path/to/content

# Additive workspace configuration
workspace_patches:
  workspacename:
    allowed_node_types:
      add:
        - myns:NodeType1
```

## Avoiding Content Conflicts

When multiple builtin packages are installed, they must not overwrite each other's content. RaisinDB uses several mechanisms to prevent conflicts.

### Path Namespacing Convention

**Use unique paths for all content.** Each package should namespace its content under a unique prefix:

| Package | Function Path Convention |
|---------|-------------------------|
| `ai-tools` | `/functions/lib/raisin/agent-*` |
| `raisin-auth` | `/functions/lib/raisin/auth/*` |
| `raisin-stewardship` | `/functions/lib/raisin/get-stewards`, `/functions/lib/raisin/get-wards`, etc. |

### The `provides:` Declaration

The `provides:` section documents what resources the package creates. This serves as:
1. **Documentation** for what the package installs
2. **Conflict detection** - overlapping paths indicate potential conflicts
3. **Dependency tracking** for package management

### Install Modes

When packages are installed, one of three modes controls conflict handling:

| Mode | Behavior | Used When |
|------|----------|-----------|
| `skip` | Only install content that doesn't exist | New package installation (default) |
| `overwrite` | Delete and replace all existing content | Reinstalling/resetting a package |
| `sync` | Update existing, create new, leave others alone | Updating an installed package |

For builtin packages:
- **First installation** uses `skip` mode (won't overwrite user content)
- **Updates** use `sync` mode (updates package content without touching user additions)

### Using `sync:` Filters for Fine-Grained Control

For advanced conflict handling, packages can define explicit sync filters in `manifest.yaml`. This gives precise control over which paths the package "owns" and how conflicts are resolved.

```yaml
sync:
  defaults:
    mode: merge
    on_conflict: prefer_server

  filters:
    - root: /functions/lib/raisin
      include:
        - "my-function/**"
        - "other-function/**"
      mode: merge

    - root: /functions/triggers
      include:
        - "my-trigger/**"
      mode: merge
```

## Sync Configuration Reference

### Filter Structure

Each filter targets a specific path and its children:

```yaml
filters:
  - root: /path/to/content     # Base path for this filter
    include:                    # Glob patterns to include (relative to root)
      - "subdir/**"
      - "*.yaml"
    exclude:                    # Glob patterns to exclude
      - "temp/**"
      - "*.bak"
    mode: merge                 # Sync mode for matched paths
    on_conflict: prefer_server  # How to handle conflicts
```

### Sync Modes

| Mode | Description |
|------|-------------|
| `replace` | Full replacement of target with source (default) |
| `merge` | Combine source and target, keeping both |
| `update` | Only apply changes, preserve unmodified content |

### Conflict Strategies

| Strategy | Description |
|----------|-------------|
| `ask` | Prompt user for each conflict (interactive, default) |
| `prefer_local` | Always use local version |
| `prefer_server` | Always use server/package version |
| `prefer_newer` | Use version with most recent timestamp |
| `keep_both` | Create both versions with suffix |
| `merge_properties` | Attempt property-level merge |
| `abort` | Stop sync on first conflict |

### Sync Direction

Control which way content flows:

| Direction | Description |
|-----------|-------------|
| `bidirectional` | Sync in both directions (default) |
| `local_only` | Never sync to server |
| `server_only` | Pull but never push |
| `push_only` | Push only, never pull from server |

## Workspace Patches

The `workspace_patches:` section modifies workspace configuration **additively**:

```yaml
workspace_patches:
  functions:
    default_folder_type: raisin:Folder
    allowed_node_types:
      add:                    # Note: 'add', not 'set' - appends to existing
        - myns:NewType

  raisin:access_control:
    allowed_node_types:
      add:
        - myns:AnotherType
```

Key points:
- Uses `add:` to append to existing lists (not replace)
- Multiple packages can patch the same workspace
- Patches are applied in package installation order

## Best Practices Checklist

When creating a new builtin package:

- [ ] Use unique path prefixes for all functions and triggers
- [ ] Prefix node type names with a unique namespace (e.g., `myns:TypeName`)
- [ ] Document all provided content in `provides:` section
- [ ] Use `add:` in workspace_patches to avoid overwriting other packages
- [ ] Add explicit `sync:` filters if the package needs update support
- [ ] Set `on_conflict: prefer_server` for package-owned content
- [ ] Test installation order with other builtin packages

## Existing Builtin Packages

| Package | Purpose | Key Content |
|---------|---------|-------------|
| `ai-tools` | AI/LLM functionality | AI node types, agent handlers, message triggers |
| `raisin-auth` | Authentication & roles | Default roles, anonymous user, user creation function |
| `raisin-stewardship` | Delegation system | Relation types, stewardship functions, relationship triggers |

## Troubleshooting

### Content Not Installing

If content isn't being installed:
1. Check that paths in `provides:` match actual content paths
2. Verify content exists at `content/{workspace}/{path}/node.yaml`
3. Check server logs for installation errors

### Content Overwritten by Another Package

If packages are overwriting each other:
1. Review `provides:` sections for overlapping paths
2. Add `sync:` filters to explicitly claim ownership
3. Rename paths to use package-specific prefixes

### Workspace Patches Not Applied

If workspace configuration isn't updating:
1. Ensure using `add:` not direct assignment for lists
2. Check workspace name matches exactly (including colons)
3. Verify the target workspace exists or is created first
