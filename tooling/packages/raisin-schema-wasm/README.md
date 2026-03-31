# raisin-schema-wasm

WASM bindings for RaisinDB schema and package validation. Provides real-time validation for YAML package files in the browser.

## License

This project is licensed under the Business Source License 1.1 (BSL-1.1). See the LICENSE file in the repository root for details.

## Features

- **Package Validation** - Validate entire RaisinDB packages with cross-file reference checking
- **File-Level Validation** - Validate individual YAML files (manifests, node types, workspaces, content)
- **Position-Accurate Errors** - Errors include line and column positions for editor integration
- **Auto-Fix Suggestions** - Many errors include suggested fixes that can be applied automatically
- **Built-in Types** - Includes built-in node types and workspaces for reference validation

## Supported File Types

| File Type | Location | Description |
|-----------|----------|-------------|
| Manifest | `manifest.yaml` | Package metadata (name, version, description) |
| Node Types | `nodetypes/*.yaml` | Node type definitions with properties and constraints |
| Archetypes | `archetypes/*.yaml` | Archetype definitions (base types with inheritance) |
| Element Types | `elementtypes/*.yaml` | Element type definitions |
| Workspaces | `workspaces/*.yaml` | Workspace configurations |
| Content | `content/**/*.yaml` | Content nodes with properties |

## Building

Requires Rust toolchain with wasm32-unknown-unknown target:

```bash
# Install wasm-pack if not already installed
cargo install wasm-pack

# Build the WASM package
./build.sh
```

Output is generated in the `pkg/` directory.

## API

### Package Validation

```typescript
import { validate_package } from 'raisin-schema-wasm';

// Validate entire package (cross-file reference checking)
const files = {
  'manifest.yaml': '...',
  'nodetypes/Article.yaml': '...',
  'content/home.yaml': '...'
};

const results = validate_package(files);
// { 'manifest.yaml': {...}, 'nodetypes/Article.yaml': {...}, ... }
```

### Individual File Validation

```typescript
import {
  validate_manifest,
  validate_nodetype,
  validate_archetype,
  validate_elementtype,
  validate_workspace,
  validate_content
} from 'raisin-schema-wasm';

// Validate manifest
const result = validate_manifest(yamlContent, 'manifest.yaml');

// Validate node type (with package context for reference checking)
const nodeTypeResult = validate_nodetype(
  yamlContent,
  'nodetypes/Article.yaml',
  ['cms:Page', 'cms:Article']  // package node types
);

// Validate workspace
const workspaceResult = validate_workspace(
  yamlContent,
  'workspaces/content.yaml',
  ['cms:Page'],     // package node types
  ['content']       // package workspaces
);

// Validate content
const contentResult = validate_content(
  yamlContent,
  'content/home.yaml',
  ['cms:Page'],     // package node types
  ['content']       // package workspaces
);
```

### Built-in Types

```typescript
import { get_builtin_node_types, get_builtin_workspaces } from 'raisin-schema-wasm';

// Get built-in node types (raisin:*, etc.)
const builtinTypes = get_builtin_node_types();

// Get built-in workspace names
const builtinWorkspaces = get_builtin_workspaces();
```

### Auto-Fix

```typescript
import { apply_fix } from 'raisin-schema-wasm';

// Apply a fix to YAML content
const fixedYaml = apply_fix(yamlContent, error, newValue);
```

## Types

```typescript
interface ValidationResult {
  file_type: FileType;
  errors: ValidationError[];
  is_valid: boolean;
}

interface ValidationError {
  path: string;           // JSON path to error location
  field: string;          // Field name with error
  code: string;           // Error code (e.g., "MISSING_FIELD")
  message: string;        // Human-readable message
  severity: 'error' | 'warning';
  line?: number;          // 1-based line number
  column?: number;        // 1-based column number
  fix?: Fix;              // Optional auto-fix
}

interface Fix {
  fix_type: FixType;
  description: string;
  value?: any;            // Suggested value for auto-fixes
}

type FixType =
  | 'AddField'            // Add missing field
  | 'RemoveField'         // Remove invalid field
  | 'ChangeValue'         // Change field value
  | 'NeedsInput';         // Requires user input

type FileType =
  | 'Manifest'
  | 'NodeType'
  | 'Archetype'
  | 'ElementType'
  | 'Workspace'
  | 'Content';
```

## Error Codes

### Manifest Errors
| Code | Description |
|------|-------------|
| `MISSING_FIELD` | Required field is missing |
| `INVALID_TYPE` | Field has wrong type |
| `INVALID_VALUE` | Field value is invalid |

### Node Type Errors
| Code | Description |
|------|-------------|
| `MISSING_FIELD` | Required field is missing |
| `INVALID_NAME` | Invalid node type name format |
| `UNKNOWN_EXTENDS` | Extends unknown node type |
| `UNKNOWN_ALLOWED_CHILD` | References unknown node type in allowed_children |
| `INVALID_PROPERTY_TYPE` | Invalid property type |
| `DUPLICATE_PROPERTY` | Property defined multiple times |

### Workspace Errors
| Code | Description |
|------|-------------|
| `MISSING_FIELD` | Required field is missing |
| `UNKNOWN_ROOT_TYPE` | Root type references unknown node type |

### Content Errors
| Code | Description |
|------|-------------|
| `MISSING_FIELD` | Required field is missing |
| `UNKNOWN_NODE_TYPE` | Node type is not defined |
| `INVALID_PROPERTY` | Property not defined in node type |
| `MISSING_REQUIRED_PROPERTY` | Required property is missing |

## Internal Dependencies

| Crate | Description |
|-------|-------------|
| `raisin-models` | Node type and content model definitions |
| `raisin-validation` | Validation logic and rules |
