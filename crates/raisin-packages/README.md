# raisin-packages

Package management for RaisinDB - install, browse, and manage .rap packages.

## Overview

This crate provides functionality for managing `.rap` (Raisin Archive Package) files, enabling modular content distribution, installation, and bidirectional synchronization between development environments and the RaisinDB server.

## Features

- **Package Browsing** - Browse ZIP contents without extracting
- **Manifest Parsing** - Parse and validate package manifests (YAML)
- **Installation** - Install packages with node types, content, and workspace patches
- **Uninstallation** - Clean removal with dependency checking
- **Dependency Graph** - Topological sorting and cycle detection
- **Content Validation** - Validate node type references and dependencies
- **Workspace Patching** - Apply configuration patches during install
- **Bidirectional Sync** - Synchronization with conflict resolution
- **Package Export** - Export installed content back to .rap files

## Package Structure

A `.rap` file is a ZIP archive containing:

```
manifest.yaml           # Package metadata and configuration
nodetypes/              # Node type definitions (.yaml files)
workspaces/             # Workspace configurations
content/                # Content to install (nodes, assets)
  workspace1/
    path/to/node.yaml
    path/to/node/
      code.star         # Associated files (e.g., function code)
```

## Usage

### Browse Package Contents

```rust
use raisin_packages::{PackageBrowser, EntryType};

let package_data = std::fs::read("my-package.rap")?;
let browser = PackageBrowser::new(&package_data)?;

// Get manifest
let manifest = browser.manifest()?;
println!("Package: {} v{}", manifest.name, manifest.version);

// List entries
for entry in browser.list_entries()? {
    match entry.entry_type {
        EntryType::File => println!("  File: {}", entry.path),
        EntryType::Directory => println!("  Dir:  {}", entry.path),
    }
}
```

### Install Package

```rust
use raisin_packages::PackageInstaller;

let package_data = std::fs::read("my-package.rap")?;
let installer = PackageInstaller::new(&package_data)?;

// Get content to install
let node_types = installer.get_node_types()?;
let content_nodes = installer.get_content_nodes()?;

// Install via storage layer
let result = installer.install(&storage).await?;
println!("Installed {} node types", result.node_types_registered.len());
```

### Dependency Graph

```rust
use raisin_packages::{DependencyGraph, AvailableTypes};

let mut graph = DependencyGraph::new();

// Add packages
graph.add_package(&manifest1, &browser1)?;
graph.add_package(&manifest2, &browser2)?;

// Get installation order (topological sort)
let order = graph.installation_order()?;

// Validate content references
let available = AvailableTypes::new(&installed_types);
let result = graph.validate_content(&available)?;
for warning in result.warnings {
    println!("Warning: {}", warning);
}
```

### Sync Configuration

```rust
use raisin_packages::{SyncConfig, SyncFilter, SyncMode};

let config = SyncConfig {
    filters: vec![
        SyncFilter {
            root: "/content/pages".to_string(),
            mode: SyncMode::Merge,
            rules: vec![
                "+.*".to_string(),        // Include all
                "-.*\\.tmp".to_string(),  // Exclude .tmp files
            ],
        },
    ],
    ..Default::default()
};
```

### Export Package

```rust
use raisin_packages::{PackageExporter, ExportOptions};

let exporter = PackageExporter::new(manifest);

// Add content
exporter.add_node_type(&node_type)?;
exporter.add_content_node(&node, &workspace)?;

// Export to bytes
let rap_bytes = exporter.export()?;
std::fs::write("exported.rap", rap_bytes)?;
```

## Manifest Format

```yaml
name: my-package
version: 1.0.0
title: My Package
description: A sample package
author: Your Name
license: MIT
icon: package          # Lucide icon name
color: "#6366F1"       # Hex color for UI
keywords:
  - sample
  - demo
category: content

# Dependencies on other packages
dependencies:
  - name: raisin-base
    version: ">=1.0.0"

# What this package provides
provides:
  node_types:
    - myapp:page
    - myapp:article
  workspaces:
    - content

# Workspace configuration patches
workspace_patches:
  content:
    settings:
      theme: dark

# Sync configuration (optional)
sync:
  direction: bidirectional
  conflict_strategy: newer_wins
  filters:
    - root: /content
      mode: merge
```

## Modules

| Module | Description |
|--------|-------------|
| `manifest` | Package manifest parsing and validation |
| `browser` | Browse ZIP contents without extracting |
| `installer` | Install/uninstall packages to repositories |
| `patcher` | Apply workspace configuration patches |
| `dependency_graph` | Dependency resolution and validation |
| `sync_config` | Sync filter configuration |
| `sync` | Sync status tracking and diff computation |
| `exporter` | Export installed content to .rap files |
| `error` | Package-specific error types |

## Sync Modes

| Mode | Description |
|------|-------------|
| `Replace` | Completely replace target content |
| `Merge` | Merge with existing content |
| `Update` | Update existing, don't create new |

## Conflict Strategies

| Strategy | Description |
|----------|-------------|
| `NewerWins` | Most recently modified wins |
| `LocalWins` | Local changes take precedence |
| `RemoteWins` | Server changes take precedence |
| `Manual` | Require manual resolution |

## File Status

| Status | Description |
|--------|-------------|
| `Unchanged` | File matches package source |
| `Modified` | Local modifications detected |
| `Added` | New file not in package |
| `Deleted` | File removed locally |
| `Conflicted` | Conflicting changes |

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
