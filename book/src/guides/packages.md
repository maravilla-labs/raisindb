# RAP: Raisin Archive Packages

RAP (Raisin Archive Package) is RaisinDB's packaging system for bundling, distributing, and installing content. Think of it as npm or Cargo, but for RaisinDB content -- node types, workspace configurations, and pre-built nodes can all be packaged into a single `.rap` file and installed into any repository.

This chapter covers everything you need to know about the RAP format, from creating packages to managing dependencies and synchronizing content.

## What is RAP?

A `.rap` file is a self-contained, installable bundle that can include:

- **Node type definitions** -- custom types like `blog:Article`, `ai:Agent`, or `ecommerce:Product`
- **Workspace configurations** -- pre-configured workspaces with allowed node types and folder structures
- **Content nodes** -- actual data nodes with properties and associated files (code, templates, assets)

RAP solves several problems:

- **Starter kits** -- Bootstrap a new project with a `blog-starter` or `ecommerce-starter` package that installs all the types, workspaces, and seed content you need.
- **Content distribution** -- Share node type definitions and reusable content across teams, environments, or organizations.
- **Migration** -- Move structured content between RaisinDB instances by exporting from one and installing into another.
- **Modularity** -- Break your application into composable packages with explicit dependencies. An `ai-tools` package can depend on `core-functions` and declare exactly what it provides.

## The RAP Format

### Archive Structure

A `.rap` file is a standard ZIP archive (using Deflate compression) with a well-defined internal layout:

```
my-package-1.0.0.rap
  manifest.yaml              # Package metadata and configuration (required)
  mixins/                    # Mixin definitions (installed before node types)
    myapp_SEO.yaml
    myapp_Timestamps.yaml
  nodetypes/                 # Node type definitions
    blog_Article.yaml
    blog_Category.yaml
  workspaces/                # Workspace configuration files
    blog.yaml
  content/                   # Content organized by workspace
    blog/                    # Workspace name
      posts/                 # Directory structure
        welcome-post/        # Each node is a directory
          node.yaml          # Node metadata (node_type, properties)
          index.md           # Associated files
        getting-started/
          node.yaml
          index.md
          hero.png           # Binary assets
```

Every package must contain a `manifest.yaml` at the root. The other directories are optional -- a package could provide only node types, only content, or any combination.

### Node Directories

Inside `content/`, each node is represented as a directory containing:

- `node.yaml` (required) -- Defines the node's type and properties
- Additional files -- Code files, templates, images, or any other assets associated with the node

For example, a function node might look like:

```yaml
# content/functions/lib/my-handler/node.yaml
node_type: raisin:Function
properties:
  title: My Handler
  description: Handles incoming webhook events
  runtime: node20
```

```javascript
// content/functions/lib/my-handler/index.js
export function handler(event) {
  return { status: "ok", received: event.data };
}
```

### The Manifest

The manifest is the heart of every package. Here is the complete schema:

```yaml
# Required fields
name: ai-tools                        # Unique identifier (alphanumeric, hyphens, underscores)
version: 1.0.0                        # Semantic version string

# Metadata (all optional)
title: RaisinDB AI Tools              # Human-readable display name
description: AI agents and chat       # Package description
author: RaisinDB Team                 # Package author
license: MIT                          # License identifier
icon: bot                             # Lucide icon name for UI (default: "package")
color: "#8B5CF6"                      # Hex color for UI display (default: "#6366F1")
keywords:                             # Search keywords
  - ai
  - agents
  - chat
category: ai                          # Package category
builtin: false                        # If true, auto-installed on repository creation

# Dependencies
dependencies:
  - name: core-functions
    version: ">=1.0.0"

# What this package provides
provides:
  mixins:
    - ai:Conversable
  nodetypes:
    - ai:Agent
    - ai:Chat
  workspaces:
    - functions
  content:
    - functions/lib/raisin/agent-handler

# Workspace patches (applied during install)
workspace_patches:
  functions:
    allowed_node_types:
      add:
        - ai:Agent
        - ai:Chat
    default_folder_type: "raisin:Folder"

# Sync configuration (optional)
sync:
  remote:
    url: "https://raisindb.example.com"
    repo_id: "my-project"
    branch: "main"
    tenant_id: "default"
  defaults:
    mode: replace
    on_conflict: ask
    sync_deletions: true
    property_merge: shallow
  filters: []
  conflicts: {}
```

Here is what each section means:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | String | Yes | Unique package identifier. Only alphanumeric characters, hyphens, and underscores. |
| `version` | String | Yes | Semantic version string (e.g., `1.0.0`). |
| `title` | String | No | Human-readable display name. |
| `description` | String | No | Brief description of the package. |
| `author` | String | No | Package author or team. |
| `license` | String | No | License identifier (e.g., `MIT`, `Apache-2.0`). |
| `icon` | String | No | Lucide icon name for UI display. Defaults to `"package"`. |
| `color` | String | No | Hex color code for UI theming. Defaults to `"#6366F1"`. |
| `keywords` | Vec\<String\> | No | Tags for package search and discovery. |
| `category` | String | No | Classification category. |
| `builtin` | bool | No | When `true`, the package is automatically installed when a new repository is created. |
| `dependencies` | Vec\<Dependency\> | No | Packages this package depends on. |
| `provides` | Provides | No | Declares what the package contributes (mixins, node types, workspaces, content paths). |
| `workspace_patches` | HashMap\<String, WorkspacePatch\> | No | Modifications to apply to workspace configurations during install. |
| `sync` | SyncConfig | No | Configuration for bidirectional synchronization. |

### Manifest in Rust

The manifest maps directly to the `Manifest` struct:

```rust
use raisin_packages::Manifest;

// Parse from YAML
let manifest = Manifest::from_yaml(yaml_str)?;

// Parse from raw bytes
let manifest = Manifest::from_bytes(raw_bytes)?;

// Extract from a ZIP archive
let manifest = Manifest::from_zip(&mut zip_archive)?;

// Validate (checks name format, required fields)
manifest.validate()?;

// Convert to JSON properties for a raisin:Package node
let props = manifest.to_node_properties();
```

## Use Cases

### Starter Kits

A blog starter kit packages everything needed for a content-driven site:

```yaml
name: blog-starter
version: 1.0.0
title: Blog Starter Kit
description: Complete blog setup with articles, categories, and authors
icon: newspaper
color: "#3B82F6"
keywords: [blog, cms, content]
category: starter

dependencies:
  - name: base-types
    version: ">=1.0.0"

provides:
  nodetypes:
    - blog:Article
    - blog:Category
    - blog:Author
  workspaces:
    - blog
  content:
    - blog/welcome-post
    - blog/getting-started

workspace_patches:
  blog:
    allowed_node_types:
      add:
        - blog:Article
        - blog:Category
        - blog:Author
```

### Sharing Node Types Across Teams

A package can provide only type definitions -- no content, no workspaces:

```yaml
name: shared-types
version: 2.0.0
title: Shared Node Types
provides:
  nodetypes:
    - shared:Document
    - shared:MediaAsset
    - shared:Tag
```

### Function Libraries

Packages can distribute serverless functions with their code files:

```yaml
name: ai-tools
version: 1.0.0
dependencies:
  - name: core-functions
    version: ">=1.0.0"

provides:
  nodetypes:
    - ai:Agent
    - ai:Chat
  workspaces:
    - functions
  content:
    - functions/lib/raisin/agent-handler
    - functions/lib/raisin/chat-handler

workspace_patches:
  functions:
    allowed_node_types:
      add:
        - ai:Agent
        - ai:Chat
```

Each function node in the content directory includes both a `node.yaml` and the actual code files (e.g., `index.js`).

## Creating Packages

### Using the PackageExporter

The `PackageExporter` builds `.rap` files programmatically:

```rust
use raisin_packages::{
    Manifest, PackageExporter, ContentBuilder,
    ExportOptions, ExportMode, Provides,
};

// Define the manifest
let manifest = Manifest {
    name: "my-package".to_string(),
    version: "1.0.0".to_string(),
    description: Some("My custom package".to_string()),
    provides: Provides {
        mixins: vec![],
        nodetypes: vec!["custom:Widget".to_string()],
        workspaces: vec!["widgets".to_string()],
        content: vec!["widgets/default-widget".to_string()],
    },
    ..Default::default()
};

// Create the exporter
let mut exporter = PackageExporter::with_manifest(manifest);

// Add mixin definitions (written to mixins/ directory)
exporter.add_mixin(
    "custom:Styled".to_string(),
    "name: custom:Styled\ndescription: Style properties\n".to_string(),
);

// Add node type definitions
exporter.add_node_type(
    "custom:Widget".to_string(),
    "name: custom:Widget\ndescription: A reusable widget\n".to_string(),
);

// Add content using ContentBuilder
let content = ContentBuilder::new("widgets", "default-widget", "custom:Widget")
    .with_properties(serde_json::json!({
        "title": "Default Widget",
        "description": "A starter widget"
    }))
    .with_file("template.html", b"<div>Hello World</div>".to_vec())
    .build();
exporter.add_content(content);

// Build the .rap file
let result = exporter.build()?;

// result.package_data contains the ZIP bytes
// result.files_included is the total file count
// result.exported_at is the timestamp
std::fs::write("my-package-1.0.0.rap", &result.package_data)?;
```

### Export Modes

The `ExportOptions` struct controls what gets included:

```rust
use raisin_packages::{ExportOptions, ExportMode};

let options = ExportOptions {
    // ExportMode::All -- include everything
    // ExportMode::Filtered -- apply manifest sync filters (default)
    export_mode: ExportMode::Filtered,

    // Glob patterns to filter content
    filter_patterns: vec!["**/*.yaml".to_string()],

    // Include locally modified content
    include_modifications: true,

    // Override the version in the exported manifest
    new_version: Some("2.0.0".to_string()),
};

let exporter = PackageExporter::new(manifest, options);
```

When `ExportMode::Filtered` is used and the manifest has a `sync` configuration, the exporter respects the sync filter rules to decide which paths to include.

### Adding Content Files Directly

For more control, you can add individual files to content entries:

```rust
let mut exporter = PackageExporter::with_manifest(manifest);

// Add files to a specific package path
exporter.add_content_file(
    "content/functions/my-handler/",
    "node.yaml",
    b"node_type: raisin:Function\nproperties:\n  title: My Handler\n".to_vec(),
);
exporter.add_content_file(
    "content/functions/my-handler/",
    "index.js",
    b"export function handler() { return 'hello'; }".to_vec(),
);
```

### Comparing Packages

The `PackageComparator` detects differences between a package's original content and the current installed state:

```rust
use raisin_packages::{PackageBrowser, PackageComparator};

let browser = PackageBrowser::from_bytes(&package_data);
let comparator = PackageComparator::from_package(&browser)?;

// Check if a file exists in the original package
if comparator.exists_in_source("content/blog/welcome/node.yaml") {
    // Check if installed content has been modified
    let current_content = std::fs::read("path/to/installed/node.yaml")?;
    if comparator.is_modified("content/blog/welcome/node.yaml", &current_content) {
        println!("Content has drifted from the package source");
    }
}

// Iterate all source paths
for path in comparator.source_paths() {
    println!("Package contains: {}", path);
}
```

The comparator uses SHA-256 hashes to detect modifications.

## Browsing Packages

The `PackageBrowser` lets you inspect a `.rap` file without installing it:

```rust
use raisin_packages::{PackageBrowser, EntryType};

let data = std::fs::read("blog-starter-1.0.0.rap")?;
let browser = PackageBrowser::new(data);
// Or: PackageBrowser::from_bytes(&data)

// Read the manifest
let manifest = browser.manifest()?;
println!("Package: {} v{}", manifest.name, manifest.version);
println!("Description: {:?}", manifest.description);
```

### Listing Entries

```rust
// List all entries in the archive
let entries = browser.list_entries()?;
for entry in &entries {
    let kind = match entry.entry_type {
        EntryType::File => "FILE",
        EntryType::Directory => "DIR ",
    };
    println!("{} {} ({} bytes)", kind, entry.path, entry.size);
    if entry.compressed {
        println!("     compressed: {} bytes", entry.compressed_size);
    }
}

// List entries in a specific directory
let content_entries = browser.list_directory("content")?;

// List only node type definitions
let nodetypes = browser.list_nodetypes()?;
for path in &nodetypes {
    println!("Node type: {}", path);
}

// List content workspaces
let workspaces = browser.list_content_workspaces()?;
for ws in &workspaces {
    println!("Workspace: {}", ws);
}
```

### Reading Files

```rust
// Read a file as bytes
let raw = browser.read_file("nodetypes/blog_Article.yaml")?;

// Read a file as UTF-8 string
let content = browser.read_file_string("manifest.yaml")?;

// Check if a path exists
if browser.exists("content/blog/welcome/node.yaml")? {
    println!("Welcome post is included");
}
```

Each `ZipEntry` provides:

| Field | Type | Description |
|-------|------|-------------|
| `path` | `String` | Path within the archive |
| `entry_type` | `EntryType` | `File` or `Directory` |
| `size` | `u64` | Uncompressed file size (0 for directories) |
| `compressed` | `bool` | Whether the file uses compression |
| `compressed_size` | `u64` | Compressed size on disk |

## Installing Packages

The `PackageInstaller` parses a `.rap` file and extracts everything needed for installation.

### Creating an Installer

```rust
use raisin_packages::PackageInstaller;

let package_data = std::fs::read("ai-tools-1.0.0.rap")?;
let installer = PackageInstaller::new(package_data)?;

// The constructor validates the manifest automatically
let manifest = installer.manifest();
println!("Installing {} v{}", manifest.name, manifest.version);
```

### Extracting Mixins

Mixins must be installed **before** node types, since node types may reference them in their `mixins` field. The `get_mixins()` method reads definitions from the `mixins/` directory:

```rust
// Get all mixin definitions from mixins/ directory
let mixins = installer.get_mixins()?;
for (name, definition) in &mixins {
    println!("Mixin: {}", name);
    // Register the mixin before installing node types
}
```

### Extracting Node Types

```rust
// Get all node type definitions from nodetypes/ directory
let node_types = installer.get_node_types()?;
for (name, definition) in &node_types {
    println!("Node type: {}", name);
    // definition is a serde_json::Value parsed from YAML
}
```

### Extracting Content

```rust
use raisin_packages::ContentNode;

// Get all content nodes from content/ directory
let content_nodes = installer.get_content()?;
for node in &content_nodes {
    println!("Workspace: {}, Path: {}, Type: {}",
        node.workspace, node.path, node.node_type);
    println!("  Properties: {}", node.properties);

    // Associated files (code, templates, assets)
    for (filename, data) in &node.files {
        println!("  File: {} ({} bytes)", filename, data.len());
    }
}
```

Each `ContentNode` contains:

| Field | Type | Description |
|-------|------|-------------|
| `workspace` | `String` | Target workspace name |
| `path` | `String` | Node path within the workspace |
| `node_type` | `String` | Node type identifier (e.g., `raisin:Function`) |
| `properties` | `serde_json::Value` | Node properties as JSON |
| `children` | `Vec<ContentNode>` | Child nodes (for hierarchical content) |
| `files` | `HashMap<String, Vec<u8>>` | Associated files (filename to raw bytes) |

### Applying Workspace Patches

```rust
// Get the workspace patcher from the manifest
let patcher = installer.get_patcher();

// See which workspaces need patching
for workspace in patcher.workspaces_to_patch() {
    println!("Needs patching: {}", workspace);
}

// Apply patches to a workspace configuration
let config = serde_json::json!({
    "name": "functions",
    "allowed_node_types": ["raisin:Function", "raisin:Trigger"]
});

let patched = patcher.apply_patches("functions", config)?;
// patched now includes the added node types from the package
```

### Recording Results

After performing the actual installation (creating nodes, registering types), record the result:

```rust
let result = installer.create_install_result(
    vec!["ai:Conversable".to_string()],                     // registered mixins
    vec!["ai:Agent".to_string(), "ai:Chat".to_string()],    // registered types
    vec!["functions".to_string()],                           // patched workspaces
    vec!["functions/lib/raisin/agent-handler".to_string()],  // created content
);

println!("Installed at: {}", result.installed_at);
println!("Mixins: {:?}", result.mixins_registered);
println!("Types: {:?}", result.node_types_registered);
println!("Workspaces: {:?}", result.workspaces_patched);
println!("Content: {:?}", result.content_nodes_created);
```

### Uninstall Results

```rust
let uninstall = installer.create_uninstall_result(
    vec!["functions/lib/raisin/agent-handler".to_string()],
    false, // node_types_removed: false if other packages use them
);

println!("Removed {} content nodes", uninstall.content_nodes_removed.len());
if !uninstall.node_types_removed {
    println!("Node types kept (used by other packages)");
}
```

## Dependencies

Packages can declare dependencies on other packages. The `DependencyGraph` handles validation, circular dependency detection, and installation ordering.

### Declaring Dependencies

In the manifest:

```yaml
dependencies:
  - name: core-functions
    version: ">=1.0.0"
  - name: base-types
    version: ">=2.0.0"
```

Each dependency has:

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Name of the required package |
| `version` | `String` | Semver version constraint |

### Building a Dependency Graph

```rust
use raisin_packages::{DependencyGraph, Manifest, Dependency};

let mut graph = DependencyGraph::new();

// Add packages (order doesn't matter)
graph.add_package(Manifest {
    name: "base-types".into(),
    version: "1.0.0".into(),
    ..Default::default()
});

graph.add_package(Manifest {
    name: "core-functions".into(),
    version: "1.0.0".into(),
    dependencies: vec![Dependency {
        name: "base-types".into(),
        version: ">=1.0.0".into(),
    }],
    ..Default::default()
});

graph.add_package(Manifest {
    name: "ai-tools".into(),
    version: "1.0.0".into(),
    dependencies: vec![Dependency {
        name: "core-functions".into(),
        version: ">=1.0.0".into(),
    }],
    ..Default::default()
});
```

### Validating Dependencies

```rust
// Check that all declared dependencies exist in the graph
graph.validate_dependencies()?;
// Returns Err(DependencyGraphError::MissingDependency { .. }) if a dep is missing
```

### Installation Order

The graph uses Kahn's algorithm (topological sort) to determine the correct installation order -- dependencies are installed before the packages that need them:

```rust
let order = graph.installation_order()?;
// Returns: ["base-types", "core-functions", "ai-tools"]

// Or get the full manifests in order:
let manifests = graph.manifests_in_order()?;
for manifest in manifests {
    println!("Install: {} v{}", manifest.name, manifest.version);
}
```

### Circular Dependency Detection

If packages form a cycle, the graph detects it and reports the exact cycle path:

```rust
let mut graph = DependencyGraph::new();
graph.add_package(/* A depends on B */);
graph.add_package(/* B depends on C */);
graph.add_package(/* C depends on A */);

match graph.installation_order() {
    Err(DependencyGraphError::CircularDependency { cycle }) => {
        // cycle contains: ["A", "B", "C", "A"]
        println!("Circular dependency: {}", cycle.join(" -> "));
    }
    _ => {}
}
```

The error message includes a visual representation of the cycle:

```
Circular dependency detected in package dependencies:
  > A
    |
    v
    B
    |
    v
    C
    |
    v
  > A (cycle)

To resolve: Remove one of these dependency relationships.
```

### Diamond Dependencies

Diamond dependency patterns (where two packages depend on the same base package) are handled correctly:

```
    A
   / \
  B   C
   \ /
    D
```

The topological sort ensures `D` is installed first, then `B` and `C` (in any order), then `A`.

### Graph Queries

```rust
// Check if a package is in the graph
graph.contains("ai-tools"); // true

// Get a specific package node
if let Some(node) = graph.get("ai-tools") {
    println!("Version: {}", node.version);
    println!("Dependencies: {:?}", node.dependencies);
}

// List all package names
let names = graph.package_names();
```

## Workspace Patches

Workspace patches let a package modify workspace configurations during installation -- typically to register new node types that the package provides.

### Defining Patches

In the manifest, `workspace_patches` is a map of workspace name to patch configuration:

```yaml
workspace_patches:
  functions:
    allowed_node_types:
      add:
        - ai:Agent
        - ai:Chat
    default_folder_type: "raisin:Folder"
  content:
    allowed_node_types:
      add:
        - blog:Article
```

### Patch Operations

Currently, workspace patches support one operation:

- **`AddAllowedNodeTypes`** -- Appends node types to a workspace's `allowed_node_types` list, avoiding duplicates.

```rust
use raisin_packages::{WorkspacePatcher, PatchOperation};

let patcher = installer.get_patcher();

// Inspect operations for a workspace
if let Some(ops) = patcher.get_patches("functions") {
    for op in ops {
        match op {
            PatchOperation::AddAllowedNodeTypes(types) => {
                println!("Adding node types: {:?}", types);
            }
        }
    }
}
```

### Applying Patches

The patcher takes a workspace configuration as JSON and returns the modified version:

```rust
let config = serde_json::json!({
    "name": "functions",
    "allowed_node_types": ["raisin:Function"]
});

let patched = patcher.apply_patches("functions", config)?;
// Result: { "name": "functions", "allowed_node_types": ["raisin:Function", "ai:Agent", "ai:Chat"] }
```

Patches are idempotent -- applying the same patch twice does not create duplicate entries.

### Reverse Patches

Currently, automatic reverse patching for uninstallation is not supported. When a package is uninstalled, the node types it added to workspace configurations remain. This is by design -- removing types could break other packages or user-created content that depends on those types.

## Package Sync

The sync system enables bidirectional synchronization between package content and a RaisinDB server. This is useful when content evolves independently in both places and you need to reconcile changes.

### Sync Status

Check whether installed content has drifted from the package source:

```rust
use raisin_packages::{PackageSyncStatus, OverallSyncStatus, SyncSummary, SyncFileStatus};
```

Each file gets one of these statuses:

| Status | Meaning |
|--------|---------|
| `SyncFileStatus::Synced` | File is identical locally and on the server |
| `SyncFileStatus::LocalOnly` | File exists only locally (added after install) |
| `SyncFileStatus::ServerOnly` | File exists only in the package (deleted locally) |
| `SyncFileStatus::Modified` | File has been modified locally |
| `SyncFileStatus::Conflict` | Both local and server versions have changed |

### Overall Status

The `OverallSyncStatus` summarizes the state of all files:

| Status | Condition |
|--------|-----------|
| `OverallSyncStatus::Synced` | All files are in sync |
| `OverallSyncStatus::Modified` | Some files have changes but no conflicts |
| `OverallSyncStatus::Conflict` | At least one file has conflicting changes |

The overall status is derived automatically from the `SyncSummary`:

```rust
let mut summary = SyncSummary::new();
summary.add(SyncFileStatus::Synced);
summary.add(SyncFileStatus::Modified);

println!("Total files: {}", summary.total());      // 2
println!("Has issues: {}", summary.has_issues());    // true
println!("Has conflicts: {}", summary.has_conflicts()); // false

let overall = OverallSyncStatus::from(&summary);
// OverallSyncStatus::Modified
```

### File Sync Information

Each file's sync state is tracked with detailed metadata:

```rust
use raisin_packages::SyncFileInfo;

// SyncFileInfo contains:
// - path: String              -- path relative to package root
// - status: SyncFileStatus    -- current sync status
// - workspace: String         -- containing workspace
// - local_hash: Option<String>    -- SHA-256 of local content
// - server_hash: Option<String>   -- SHA-256 of server content
// - local_modified_at: Option<DateTime<Utc>>
// - server_modified_at: Option<DateTime<Utc>>
// - node_type: Option<String>     -- node type if applicable
```

### File Diffs

For conflict resolution, the `FileDiff` type provides detailed difference information:

```rust
use raisin_packages::{FileDiff, DiffType};

// DiffType::Text   -- text content that can be diffed line-by-line
// DiffType::Binary -- binary content that cannot be diffed

// FileDiff contains:
// - path: String
// - diff_type: DiffType
// - local_content: Option<String>   -- for text files
// - server_content: Option<String>  -- for text files
// - unified_diff: Option<String>    -- unified diff format
```

### Sync Results

After a sync operation completes, the `SyncResult` reports what happened:

```rust
use raisin_packages::SyncResult;

let result = SyncResult::new();

// After sync operations populate the result:
println!("Uploaded: {:?}", result.uploaded);
println!("Downloaded: {:?}", result.downloaded);
println!("Conflicts: {}", result.conflicts.len());
println!("Skipped: {:?}", result.skipped);
println!("Errors: {:?}", result.errors);

println!("Success: {}", result.is_success());       // no errors or conflicts
println!("Total synced: {}", result.total_synced()); // uploaded + downloaded
```

### Content Hashing

Sync uses SHA-256 hashing to detect changes efficiently:

```rust
use raisin_packages::compute_hash;

let hash = compute_hash(b"hello world");
// Returns: "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
```

## Sync Configuration

The sync system is highly configurable through the manifest's `sync` section. Configuration follows a layered approach: defaults apply globally, filters override for specific paths, and conflict overrides target individual files.

### Remote Configuration

```yaml
sync:
  remote:
    url: "https://raisindb.example.com"
    repo_id: "my-project"
    branch: "main"                    # default: "main"
    tenant_id: "default"              # default: "default"
    auth_profile: "production"        # optional credential profile
    headers:                          # optional custom headers
      X-Custom-Header: "value"
```

### Sync Defaults

Default settings apply to all paths unless overridden by a filter:

```yaml
sync:
  defaults:
    mode: replace           # replace | merge | update
    on_conflict: ask        # ask | prefer_local | prefer_server | prefer_newer | keep_both | merge_properties | abort
    sync_deletions: true    # whether to propagate deletions
    property_merge: shallow # shallow | deep
```

**Sync modes:**

| Mode | Behavior |
|------|----------|
| `Replace` | Full replacement of target with source (default) |
| `Merge` | Combine source and target, keeping content from both |
| `Update` | Only apply changes, preserve unmodified content |

**Conflict strategies:**

| Strategy | Behavior |
|----------|----------|
| `Ask` | Prompt for user input on each conflict (default) |
| `PreferLocal` | Always keep the local version |
| `PreferServer` | Always keep the server version |
| `PreferNewer` | Use whichever version has the more recent timestamp |
| `KeepBoth` | Create both versions with a distinguishing suffix |
| `MergeProperties` | Attempt property-level merge of the conflicting content |
| `Abort` | Stop the sync operation on the first conflict |

### Sync Filters

Filters provide path-specific overrides. They are evaluated in order, and the **last matching filter wins**:

```yaml
sync:
  filters:
    # Merge YAML content in /content/pages, exclude drafts
    - root: /content/pages
      mode: merge
      include:
        - "**/*.yaml"
      exclude:
        - "drafts/**"
        - "**/.local"
      on_conflict: prefer_newer
      properties:
        include: [title, body, status]
        exclude: [internal_notes]

    # Local development content, never sync to server
    - root: /content/dev
      direction: local_only

    # System content, pull only
    - root: /system
      direction: server_only
      on_conflict: prefer_server

    # Cleanup orphaned test data
    - root: /test-data
      type: cleanup
```

Each filter can override:

| Field | Type | Description |
|-------|------|-------------|
| `root` | `String` | Path prefix this filter applies to (required) |
| `mode` | `SyncMode` | Override sync mode (`replace`, `merge`, `update`) |
| `direction` | `SyncDirection` | Override sync direction |
| `type` | `FilterType` | `normal` (default) or `cleanup` |
| `include` | `Vec<String>` | Glob patterns -- path must match at least one |
| `exclude` | `Vec<String>` | Glob patterns -- path must not match any |
| `on_conflict` | `ConflictStrategy` | Override conflict strategy |
| `properties` | `PropertyFilter` | Property-level filtering for merges |

**Sync directions:**

| Direction | Push | Pull | Description |
|-----------|------|------|-------------|
| `Bidirectional` | Yes | Yes | Full two-way sync (default) |
| `LocalOnly` | No | No | Local development content, never synced |
| `ServerOnly` | No | Yes | Pull from server, never push |
| `PushOnly` | Yes | No | Push to server, never pull |

### Glob Patterns

Filters use glob pattern matching with these wildcards:

| Pattern | Matches |
|---------|---------|
| `*` | Any sequence of characters within a single path segment |
| `**` | Any number of path segments (zero or more) |
| `?` | Any single character |

Examples:
- `*.yaml` matches `page.yaml` but not `dir/page.yaml`
- `**/*.yaml` matches `page.yaml`, `dir/page.yaml`, and `dir/sub/page.yaml`
- `drafts/**` matches everything under `drafts/`
- `test?.yaml` matches `test1.yaml` but not `test12.yaml`

### Property Filtering

For merge operations, you can control which properties are synced:

```yaml
properties:
  include: [title, body, status]         # Only sync these properties
  exclude: [internal_notes, debug_info]  # Never sync these
  preserve_local: [custom_css]           # Always keep local values
  preserve_server: [canonical_url]       # Always keep server values
  merge_keys:                            # Keys for array merging by ID
    items: "id"
  merge_strategy:
    arrays: concat      # concat | replace | unique | merge_by_key
    objects: shallow     # shallow | deep | replace
    scalars: prefer_local # prefer_local | prefer_server | prefer_newer
```

**Array merge modes:**

| Mode | Behavior |
|------|----------|
| `Concat` | Append source arrays to target (default) |
| `Replace` | Replace target array entirely |
| `Unique` | Combine arrays, removing duplicates |
| `MergeByKey` | Match array items by a key field and merge individually |

**Object merge modes:**

| Mode | Behavior |
|------|----------|
| `Shallow` | Merge top-level keys only (default) |
| `Deep` | Recursive merge of nested objects |
| `Replace` | Replace the entire object |

**Scalar merge modes:**

| Mode | Behavior |
|------|----------|
| `PreferLocal` | Keep the local value (default) |
| `PreferServer` | Keep the server value |
| `PreferNewer` | Keep whichever is newer by timestamp |

### Conflict Overrides

For specific paths that need special handling, use explicit conflict overrides:

```yaml
sync:
  conflicts:
    "/content/pages/home":
      strategy: prefer_local
      backup: true                 # Create backup before overwriting
      merge_arrays: unique         # Optional array merge mode for this path
    "/content/pages/about":
      strategy: prefer_server
      backup: false
```

### Sync Hooks

Lifecycle hooks run shell commands at key points during sync:

```yaml
sync:
  hooks:
    before_sync:
      - "echo 'Starting sync...'"
      - "npm run validate"
    after_sync:
      - "npm run build"
    on_conflict:
      - "notify-team 'Sync conflict detected'"
```

### Effective Configuration Resolution

The sync system resolves configuration by checking in this order (most specific wins):

1. **Conflict overrides** (`sync.conflicts`) -- exact path matches
2. **Filters** (`sync.filters`) -- last matching filter by path prefix
3. **Defaults** (`sync.defaults`) -- fallback for everything else

```rust
use raisin_packages::SyncConfig;

let config: SyncConfig = serde_yaml::from_str(yaml)?;

// Resolve effective settings for a path
let mode = config.get_mode_for_path("/content/pages/home.yaml");
let direction = config.get_direction_for_path("/content/dev/test.yaml");
let strategy = config.get_conflict_strategy_for_path("/content/pages/home");

// Check if a path should be synced at all
if config.should_sync_path("/content/pages/article.yaml") {
    // Proceed with sync
}
```

## Content Validation

Before installing a package, you can validate that all type references in its content are resolvable -- either provided by the package itself or already present in the database.

### Available Types

Build a registry of known types:

```rust
use raisin_packages::{AvailableTypes, ContentValidator, Manifest};

let mut available = AvailableTypes::new();

// Add types from the package manifest
available.add_from_manifest(&manifest);

// Add types already in the database
available.add_node_type("raisin:Function");
available.add_node_type("raisin:Trigger");
available.add_mixin("raisin:publishable");
available.add_archetype("base:Document");
available.add_element_type("ui:Button");

// Merge types from another source
let mut other = AvailableTypes::new();
other.add_node_type("shared:Widget");
available.merge(&other);

// Query
available.has_node_type("raisin:Function");   // true
available.has_mixin("raisin:publishable");     // true
available.has_archetype("base:Document");       // true
available.has_element_type("ui:Button");        // true
```

The `AvailableTypes` struct tracks four categories:

| Category | Description |
|----------|-------------|
| `node_types` | Node type identifiers (e.g., `blog:Article`) |
| `mixins` | Mixin identifiers (e.g., `raisin:publishable`) |
| `archetypes` | Archetype identifiers (e.g., `base:Document`) |
| `element_types` | Element type identifiers (e.g., `ui:Button`) |

### Running Validation

```rust
use raisin_packages::ContentValidator;

let validator = ContentValidator::new()
    .with_package_types(package_types)
    .with_database_types(database_types);

// Validate individual references
if let Some(warning) = validator.validate_node_type("blog:Article", "content/blog/post/node.yaml") {
    println!("Warning: {}", warning);
}

if let Some(warning) = validator.validate_archetype("unknown:Type", "content/test/node.yaml") {
    println!("Warning: {}", warning);
}

if let Some(warning) = validator.validate_element_type("ui:Missing", "content/page/node.yaml") {
    println!("Warning: {}", warning);
}

if let Some(warning) = validator.validate_mixin("unknown:Mixin", "nodetypes/blog_Article.yaml") {
    println!("Warning: {}", warning);
}

// Check availability directly
validator.is_node_type_available("blog:Article");   // checks both package and database
validator.is_mixin_available("raisin:publishable");
validator.is_archetype_available("base:Document");
validator.is_element_type_available("ui:Button");
```

### Validation Results

Collect validation findings into a `ContentValidationResult`:

```rust
use raisin_packages::{ContentValidationResult, ContentValidationWarning};

let mut result = ContentValidationResult::new();

// Record type references found in content
result.add_node_type_reference("blog:Article");
result.add_mixin_reference("myapp:SEO");
result.add_archetype_reference("base:Document");
result.add_element_type_reference("ui:Button");

// Add warnings for missing types
result.add_warning(ContentValidationWarning {
    file_path: "content/blog/post/node.yaml".to_string(),
    reference_type: "node_type".to_string(),
    type_name: "blog:Missing".to_string(),
    message: "Node type not found in package. It may exist in the database.".to_string(),
});

if result.has_warnings() {
    for warning in &result.warnings {
        println!("{}", warning);
        // Output: [content/blog/post/node.yaml] node_type: node_type 'blog:Missing' - Node type not found...
    }
}

// Merge results from multiple validation passes
let mut combined = ContentValidationResult::new();
combined.merge(result);
```

Validation is advisory -- warnings indicate types that aren't in the package or the known database types, but they may still resolve at runtime if the types exist elsewhere.

## Error Handling

All package operations return `PackageResult<T>`, which wraps `PackageError`:

| Error | When |
|-------|------|
| `InvalidPackage(String)` | General package format issues |
| `ManifestNotFound` | ZIP archive has no `manifest.yaml` |
| `InvalidManifest(String)` | Manifest parsing or validation failures |
| `FileNotFound(String)` | Requested file doesn't exist in the archive |
| `ZipError` | Underlying ZIP format issues |
| `IoError` | I/O failures during read/write |
| `YamlError` | YAML parsing failures |
| `JsonError` | JSON parsing failures |
| `AlreadyInstalled(String)` | Package is already installed |
| `NotInstalled(String)` | Trying to uninstall a package that isn't installed |
| `DependencyNotSatisfied(String, String)` | A required dependency is missing or wrong version |
| `NodeTypeConflict(String)` | A node type name conflicts with an existing one |
| `WorkspaceNotFound(String)` | Referenced workspace doesn't exist |
| `StorageError(String)` | Storage layer failures |

Dependency graph operations use their own error type, `DependencyGraphError`:

| Error | When |
|-------|------|
| `CircularDependency { cycle }` | Packages form a dependency cycle |
| `MissingDependency { package, dependency }` | A declared dependency isn't in the graph |
| `TypeReferenceNotFound { package, reference_type, name }` | A type reference can't be resolved |
