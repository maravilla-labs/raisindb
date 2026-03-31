# raisin-models

Core data models and type definitions for RaisinDB.

## Overview

This crate contains all the core data structures used throughout RaisinDB, including nodes, node types, properties, workspaces, authentication models, and permissions. It serves as the foundation for the entire database system.

## Features

- **Node System** - Hierarchical content nodes with flexible property schemas
- **Type System** - NodeType definitions with inheritance, mixins, and validation
- **Property Values** - Rich type system supporting primitives, collections, and domain types
- **Multi-Tenancy** - Tenant and deployment registration models
- **Authentication** - Identity, session, and JWT claims models
- **Permissions** - Row-Level Security (RLS) with path patterns and conditions
- **Translations** - Multi-language content support
- **Versioning** - Node version tracking and history

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      raisin-models                          │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │    nodes    │  │    auth     │  │ permissions │         │
│  │  - Node     │  │  - Identity │  │ - Permission│         │
│  │  - NodeType │  │  - Session  │  │ - Condition │         │
│  │  - Property │  │  - Claims   │  │ - PathMatch │         │
│  └─────────────┘  └─────────────┘  └─────────────┘         │
│                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │  workspace  │  │  registry   │  │ translations│         │
│  │  - Config   │  │  - Tenant   │  │ - Metadata  │         │
│  │  - Delta    │  │  - Deploy   │  │ - Helpers   │         │
│  └─────────────┘  └─────────────┘  └─────────────┘         │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## Core Types

### Node

The primary content entity in RaisinDB's hierarchical structure:

```rust
use raisin_models::nodes::Node;

let node = Node {
    id: "node-123".to_string(),
    name: "My Page".to_string(),
    path: "/content/my-page".to_string(),
    node_type: "raisin:page".to_string(),
    properties: HashMap::new(),
    // ... other fields
};
```

### PropertyValue

Rich value type system for node properties:

```rust
use raisin_models::nodes::properties::PropertyValue;

// Primitives
PropertyValue::Null
PropertyValue::Boolean(true)
PropertyValue::Integer(42)
PropertyValue::Float(3.14)
PropertyValue::String("hello".to_string())
PropertyValue::Date(timestamp)
PropertyValue::Decimal(dec!(123.456))

// Domain types
PropertyValue::Reference(RaisinReference { ... })
PropertyValue::Url(RaisinUrl { ... })
PropertyValue::Resource(Resource { ... })

// Collections
PropertyValue::Array(vec![...])
PropertyValue::Object(HashMap::new())
PropertyValue::Vector(vec![0.1, 0.2, 0.3])  // Embeddings
PropertyValue::Geometry(GeoJson { ... })     // Geospatial
```

### NodeType

Schema definitions for nodes with inheritance:

```rust
use raisin_models::nodes::types::NodeType;

let node_type = NodeType {
    name: "myapp:article".to_string(),
    extends: Some("raisin:page".to_string()),
    mixins: vec!["raisin:publishable".to_string()],
    properties: Some(vec![
        PropertyValueSchema { name: "title", ... },
        PropertyValueSchema { name: "content", ... },
    ]),
    allowed_children: vec!["myapp:comment".to_string()],
    // ... other fields
};
```

### Workspace

Isolated content containers:

```rust
use raisin_models::workspace::Workspace;

let workspace = Workspace {
    id: "ws-123".to_string(),
    name: "Production".to_string(),
    settings: WorkspaceSettings::default(),
    // ...
};
```

## Modules

| Module | Description |
|--------|-------------|
| `nodes` | Node, DeepNode, and hierarchical structure |
| `nodes::types` | NodeType, ElementType, BlockType definitions |
| `nodes::properties` | PropertyValue, PropertySchema, validation |
| `nodes::version` | NodeVersion for history tracking |
| `nodes::audit_log` | Audit trail models |
| `auth` | AuthContext, Identity, Session, AuthClaims |
| `permissions` | Permission, RoleCondition, ResolvedPermissions |
| `workspace` | Workspace configuration and delta operations |
| `registry` | TenantRegistration, DeploymentRegistration |
| `translations` | Multi-language content metadata |
| `tree` | Tree traversal utilities |
| `timestamp` | StorageTimestamp with epoch detection |
| `fractional_index` | Ordering keys for siblings |
| `operations` | Operation types for mutations |
| `errors` | Model validation errors |
| `api_key` | API key models |
| `admin_user` | Admin user models |
| `migrations` | Schema migration models |

## Property Value Types

| Type | Description | Example |
|------|-------------|---------|
| `Null` | Explicit null | `null` |
| `Boolean` | True/false | `true` |
| `Integer` | 64-bit integer | `42` |
| `Float` | Double precision | `3.14` |
| `Decimal` | 128-bit decimal | `"123.456789"` |
| `String` | UTF-8 text | `"hello"` |
| `Date` | RFC3339 timestamp | `"2024-01-15T10:30:00Z"` |
| `Reference` | Cross-node reference | `{"raisin:ref": {...}}` |
| `Url` | Rich URL with metadata | `{"raisin:url": {...}}` |
| `Resource` | File attachment | `{"id": "...", "url": "..."}` |
| `Composite` | Structured blocks | `{"items": [...]}` |
| `Element` | Typed element | `{"type": "...", "content": {...}}` |
| `Vector` | f32 array (embeddings) | `[0.1, 0.2, 0.3]` |
| `Geometry` | GeoJSON | `{"type": "Point", ...}` |
| `Array` | Heterogeneous array | `[1, "two", true]` |
| `Object` | Key-value map | `{"key": "value"}` |

## NodeType Features

| Field | Description |
|-------|-------------|
| `extends` | Single inheritance from parent type |
| `mixins` | Multiple mixin composition |
| `properties` | Property schema definitions |
| `allowed_children` | Permitted child node types |
| `required_nodes` | Required child nodes |
| `initial_structure` | Default child nodes on creation |
| `versionable` | Enable version history |
| `publishable` | Enable publish workflow |
| `auditable` | Enable audit logging |
| `indexable` | Enable search indexing |
| `index_types` | Which index types to use |

## Serialization

All models support both JSON and MessagePack serialization:

```rust
// JSON (human-readable)
let json = serde_json::to_string(&node)?;

// MessagePack (binary, efficient)
let msgpack = rmp_serde::to_vec(&node)?;
```

Timestamps use format-aware serialization:
- **JSON**: RFC3339 strings (`"2024-01-15T10:30:00Z"`)
- **MessagePack**: i64 nanoseconds (compact binary)

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
