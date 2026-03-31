# Database Comparisons

**Last Updated**: October 16, 2025

## Overview

RaisinDB is a **general-purpose application database** with built-in versioning, branching, and structured data management. Almost every modern application—whether web, mobile, desktop, or voice-based—needs to manage versioned data, track changes, handle drafts, and maintain audit trails. RaisinDB provides these primitives out-of-the-box, saving you weeks or months of custom development.

This page compares RaisinDB to popular database systems and shows how it eliminates common development overhead.

---

## RaisinDB vs. MongoDB

| Aspect | MongoDB | RaisinDB |
|--------|---------|----------|
| **Data Model** | Flexible documents (BSON) | Typed nodes with schema validation |
| **Versioning** | Manual (application-level) | Built-in immutable snapshots per revision |
| **Branching** | Not supported | Native branches with isolation |
| **References** | DBRef or manual ID references | First-class `Reference` property type with path tracking |
| **History** | Requires custom implementation | Native time-travel queries to any revision |
| **Queries** | Powerful aggregation pipeline | Type-aware queries + property filters + path lookups |
| **Storage** | WiredTiger (B-tree) | RocksDB (LSM tree) - optimized for writes |
| **Use Case** | General-purpose document store | Applications requiring versioning and structured data |

### When to choose RaisinDB over MongoDB

- ✅ Building applications with draft/production workflows
- ✅ Need audit trails and change history
- ✅ Require reference integrity tracking between entities
- ✅ Want type-safe schemas with validation
- ✅ Need to query historical data states

### When to choose MongoDB over RaisinDB

- ✅ Need proven sharding and distributed writes
- ✅ Prefer completely schema-less flexibility
- ✅ Don't need versioning or branching
- ✅ Require mature ecosystem and tooling
- ✅ Need proven scaling to petabyte-scale datasets

---

## RaisinDB vs. PostgreSQL (with JSONB)

| Aspect | PostgreSQL + JSONB | RaisinDB |
|--------|-------------------|----------|
| **Data Model** | Relational + JSON columns | Document-oriented nodes |
| **Versioning** | Temporal tables (manual setup) | Native immutable snapshots |
| **Branching** | Not supported | Native workspace deltas + branches |
| **Type System** | SQL types + JSON validation | Rich property types (Reference, Block, Resource) |
| **References** | Foreign keys (relational only) | Cross-document references with path resolution |
| **Nested Data** | JSONB (indexable, but flat) | Hierarchical trees with structural sharing |
| **History Queries** | Temporal queries (if configured) | Revision-based time-travel |
| **ACID** | Full ACID compliance | Transactional within branches |
| **Use Case** | Relational data + some JSON | Hierarchical data with versioning |

### When to choose RaisinDB over PostgreSQL

- ✅ Building applications with hierarchical data structures
- ✅ Need native versioning without complex temporal table setup
- ✅ Want branch-based workflows (dev/staging/production)
- ✅ Require rich content types (blocks, references, resources)
- ✅ Data is document-oriented rather than relational

### When to choose PostgreSQL over RaisinDB

- ✅ Need complex relational joins across many tables
- ✅ Require proven replication and high availability
- ✅ Want mature ecosystem (pgAdmin, extensions, connectors)
- ✅ ACID guarantees across all operations critical
- ✅ Team expertise in SQL and relational modeling

---

## What RaisinDB Solves: Real Application Examples

## What RaisinDB Solves: Real Application Examples

Most applications require the same fundamental capabilities. RaisinDB provides these out-of-the-box, eliminating weeks or months of custom development:

### 1. Rich Property Types: Stop Building Custom Type Systems

**The Problem**: Most applications need more than just strings and numbers. You end up building custom validation, reference tracking, and type systems.

**RaisinDB Solution**: First-class support for application-level types:

```rust
pub enum PropertyType {
    String,           // Simple text
    Number,           // Numeric values
    Boolean,          // True/false
    Date,             // Timestamps
    URL,              // Validated URLs
    Reference,        // Cross-document references with path tracking ✨
    Resource,         // Media files with metadata
    Block,            // Rich content blocks (paragraphs, images, etc.)
    BlockContainer,   // Ordered list of blocks ✨
    Array,            // Lists of any type
    Object,           // Nested structures
}
```

**Example: Reference Property**

```json
{
  "hero_image": {
    "raisin:ref": "550e8400-e29b-41d4-a716-446655440000",
    "raisin:workspace": "ws1",
    "raisin:path": "/assets/images/hero.png"
  }
}
```

**Real Application Examples:**

**E-commerce Platform:**
```json
{
  "product": {
    "name": "Wireless Headphones",
    "price": 299.99,
    "images": [
      { "raisin:ref": "img-1", "raisin:path": "/media/headphones-front.jpg" },
      { "raisin:ref": "img-2", "raisin:path": "/media/headphones-side.jpg" }
    ],
    "related_products": [
      { "raisin:ref": "prod-42", "raisin:path": "/products/carrying-case" }
    ],
    "specifications": {
      "weight": 250,
      "battery_life": "30 hours",
      "warranty_expires": "2026-10-16T00:00:00Z"
    }
  }
}
```

**What you get for free:**
- ✅ Automatic reference tracking (which products use which images)
- ✅ Reverse lookups ("show all products using this image")
- ✅ Broken reference detection when assets deleted
- ✅ Cross-branch reference resolution (preview vs. production)

**Time saved**: 2-4 weeks building custom reference tracking

---

**Design Tool / Visual Editor:**
```json
{
  "screen": {
    "name": "Dashboard",
    "layout": {
      "type": "grid",
      "columns": 12,
      "components": [
        {
          "type": "chart",
          "data_source": { "raisin:ref": "ds-1", "raisin:path": "/data-sources/sales" },
          "position": { "x": 0, "y": 0, "w": 6, "h": 4 }
        }
      ]
    }
  }
}
```

**What you get for free:**
- ✅ Component reference tracking
- ✅ Asset dependency resolution
- ✅ Type-safe property validation
- ✅ "Show usage" for reusable components

**Time saved**: 3-5 weeks building component library and reference system

### 2. Versioning & Audit Trails: Stop Building Change History Tables

**The Problem**: Almost every serious application needs change history, audit trails, and "undo" functionality. You end up building custom versioning tables, triggers, and history queries.

**RaisinDB Solution**: Built-in immutable snapshots and time-travel queries.

**Real Application Examples:**

**SaaS Configuration Management:**
```rust
// Get current configuration
let config = db.get_node("/settings/email-templates").await?;

// See configuration from 2 weeks ago
let old_config = db.get_node_at_revision("/settings/email-templates", revision_14_days_ago).await?;

// Show full audit trail
let history = db.get_revision_history("/settings/email-templates").await?;
for change in history {
    println!("{}}: {} changed by {}", change.timestamp, change.path, change.actor);
}
```

**What you get for free:**
- ✅ Full change history for compliance
- ✅ "Who changed what when" audit trail
- ✅ Point-in-time recovery
- ✅ Rollback to any previous state
- ✅ Compare current vs. historical states

**Time saved**: 4-8 weeks building audit tables, triggers, and history UI

---

**Form Builder / Survey Tool:**
```rust
// User edits form in draft
db.update_node("/forms/customer-survey", updated_form).await?;

// Preview shows draft version
let draft = db.get_node("/forms/customer-survey").await?;

// Production still shows committed version
let published = db.get_node_at_revision("/forms/customer-survey", production_revision).await?;

// Publish draft to production
let new_rev = db.commit("main", "Updated survey questions").await?;
```

**What you get for free:**
- ✅ Draft/published workflows
- ✅ Preview before publishing
- ✅ Version comparison
- ✅ Rollback published forms

**Time saved**: 3-6 weeks building draft/publish system

### 3. Branch-Based Workflows: Stop Building Custom Environment Management

**The Problem**: Applications need multiple environments (development, staging, production), A/B testing variants, or per-customer customizations. You end up with complex deployment pipelines or duplicate databases.

**RaisinDB Solution**: Native branches with isolated workspace deltas.

**Real Application Examples:**

**Multi-Tenant SaaS with Customer Customizations:**
```rust
// Create customer-specific branch from main template
db.create_branch("customer-acme", from_revision: main_head).await?;

// Customer makes customizations
db.switch_branch("customer-acme");
db.update_node("/dashboard/layout", acme_custom_layout).await?;
db.commit("customer-acme", "ACME Corp custom dashboard").await?;

// Main template updated independently
db.switch_branch("main");
db.update_node("/features/new-widget", new_widget).await?;
db.commit("main", "Added analytics widget").await?;

// Merge main updates into customer branch
db.merge("main", into: "customer-acme").await?;
```

**What you get for free:**
- ✅ Per-customer customization branches
- ✅ Shared base template across customers
- ✅ Merge template updates to customer branches
- ✅ Customer-specific feature toggles
- ✅ A/B test variants in parallel branches

**Time saved**: 6-10 weeks building multi-tenant customization system

---

**Mobile App with Staged Rollouts:**
```rust
// Development branch
db.create_branch("v2.0-dev", from: "v1.5-production").await?;
db.update_node("/features/new-onboarding", dev_onboarding).await?;
db.commit("v2.0-dev", "New onboarding flow").await?;

// Beta testing branch
db.create_branch("v2.0-beta", from: "v2.0-dev").await?;

// Production still on v1.5
let prod_config = db.get_node_at_revision("/config/app", v1_5_revision).await?;

// Promote beta to production
db.merge("v2.0-beta", into: "production").await?;
```

**What you get for free:**
- ✅ Staged rollout (dev → beta → production)
- ✅ Feature flags per environment
- ✅ Instant rollback (switch branch pointer)
- ✅ Parallel development tracks

**Time saved**: 4-7 weeks building environment management

### 4. Block-Based Composition: Stop Building Custom Content Editors

**The Problem**: Modern applications need rich, composable interfaces (dashboards, forms, pages, reports). You end up building custom drag-and-drop editors, component systems, and serialization.

**RaisinDB Solution**: Native block support for composable content.

**Real Application Examples:**

**Dashboard Builder:**
```json
{
  "dashboard": {
    "uuid": "dash-sales-overview",
    "items": [
      {
        "uuid": "block-1",
        "block_type": "metric_card",
        "content": {
          "title": "Total Revenue",
          "value": 125000,
          "trend": "+12%",
          "data_source": { "raisin:ref": "ds-1", "raisin:path": "/queries/revenue" }
        }
      },
      {
        "uuid": "block-2",
        "block_type": "chart",
        "content": {
          "type": "line",
          "data_source": { "raisin:ref": "ds-2", "raisin:path": "/queries/monthly-sales" },
          "config": { "x_axis": "month", "y_axis": "sales" }
        }
      }
    ]
  }
}
```

**What you get for free:**
- ✅ Drag-and-drop reordering (just reorder `items` array)
- ✅ Block-level versioning (track changes per widget)
- ✅ Reusable block templates
- ✅ Reference tracking (which data sources used)

**Time saved**: 8-12 weeks building dashboard editor

---

**Visual Page Builder:**
```json
{
  "landing_page": {
    "uuid": "page-home",
    "items": [
      {
        "uuid": "section-hero",
        "block_type": "hero_section",
        "content": {
          "heading": "Welcome to Our Product",
          "background": { "raisin:ref": "img-hero", "raisin:path": "/media/hero-bg.jpg" },
          "cta_button": { "text": "Get Started", "link": "/signup" }
        }
      }
    ]
  }
}
```

**Time saved**: 10-15 weeks building visual editor and template system

---

## Feature Matrix

Quick reference for choosing the right database:

| Feature | MongoDB | PostgreSQL | RaisinDB |
|---------|---------|------------|----------|
| Document storage | ✅ | ⚠️ (JSONB) | ✅ |
| Relational queries | ❌ | ✅ | ❌ |
| Built-in versioning | ❌ | ⚠️ (temporal) | ✅ |
| Branching | ❌ | ❌ | ✅ |
| Type schemas | ⚠️ (validation) | ✅ (SQL) | ✅ |
| Reference tracking | ⚠️ (manual) | ⚠️ (FK only) | ✅ |
| Hierarchical trees | ❌ | ⚠️ (recursive) | ✅ |
| Time-travel queries | ❌ | ⚠️ (temporal) | ✅ |
| Rich content blocks | ❌ | ❌ | ✅ |
| Distributed/HA | ✅ | ✅ | 🔜 (planned) |
| ACID transactions | ✅ | ✅ | ⚠️ (per-branch) |
| Mature ecosystem | ✅ | ✅ | ❌ (new) |

**Legend:**
- ✅ Full support
- ⚠️ Partial support or requires manual setup
- ❌ Not supported
- 🔜 Planned for future release

---

## Application Decision Guide

### Choose RaisinDB if you're building:

1. **SaaS Applications with Multi-Tenancy**
   - Per-customer customization branches
   - Shared base template with customer overrides
   - Environment isolation (dev/staging/production)
   - Audit trails for compliance

2. **Visual / No-Code Builders**
   - Dashboard builders, form designers, page editors
   - Drag-and-drop interfaces with blocks
   - Template libraries and reusable components
   - Preview/publish workflows

3. **Configuration & Settings Management**
   - Versioned application configuration
   - Environment-specific settings (dev/staging/prod)
   - Change history and rollback capability
   - Audit trails ("who changed what when")

4. **E-commerce & Product Catalogs**
   - Product data with rich media references
   - Seasonal catalogs or A/B test variants
   - Multi-language product information
   - Preview changes before going live

5. **Design Tools & Creative Applications**
   - Asset libraries with reference tracking
   - Component-based composition
   - Version history for designs
   - Collaboration with draft/review workflows

6. **Internal Tools & Admin Panels**
   - Dynamic forms and workflows
   - Hierarchical data structures
   - Change auditing for compliance
   - Rollback to previous states

### Choose MongoDB if:

- General-purpose document store without versioning needs
- Mature ecosystem and tooling critical
- Need sharding and distributed writes today
- Prefer completely schema-less flexibility

### Choose PostgreSQL if:

- Data is primarily relational (customers, orders, transactions)
- Need complex joins across many tables
- Full ACID guarantees across all operations critical
- Team expertise in SQL and relational modeling

---

## Performance Comparison

Approximate benchmarks (single-node, 20,000 nodes):

| Operation | MongoDB | PostgreSQL | RaisinDB |
|-----------|---------|------------|----------|
| Read single node | 1-2ms | 1-3ms | 1-2ms |
| Write single node | 2-5ms | 3-8ms | 2-5ms |
| Commit (snapshot) | N/A | N/A | 50-100ms* |
| Query by type | 10-20ms | 15-30ms | 10-20ms |
| Tree traversal | 50-100ms | 100-200ms | 20-50ms** |
| Branch creation | N/A | N/A | &lt;1ms |
| Time-travel query | N/A | 50-100ms | 10-20ms |

**Notes:**
- *With structural sharing optimization (20x faster than naive)
- **Native tree storage vs. recursive queries
- Benchmarks are approximate and workload-dependent

---

## Migration Paths

### From MongoDB to RaisinDB

**Best for:** CMS projects that outgrew MongoDB's lack of versioning

**Steps:**
1. Export MongoDB documents to JSON
2. Define RaisinDB NodeTypes from document schemas
3. Import documents as nodes with initial revision
4. Set up branch structure (main, staging, etc.)
5. Migrate application code to RaisinDB API

**Complexity:** Medium (schema definition required)

### From PostgreSQL to RaisinDB

**Best for:** Applications moving from relational to document-oriented with versioning

**Steps:**
1. Export hierarchical data (e.g., nested JSON)
2. Define NodeTypes for each data type
3. Convert foreign keys to RaisinDB references
4. Import as tree structure with revisions
5. Update queries to use RaisinDB API

**Complexity:** Medium-High (denormalization required)

---

## Conclusion

RaisinDB is a **general-purpose application database** that eliminates common development overhead:

**Save Development Time:**
- ✅ **2-4 weeks**: Reference tracking and asset management
- ✅ **4-8 weeks**: Audit trails and change history
- ✅ **6-10 weeks**: Multi-tenant customization systems
- ✅ **8-12 weeks**: Visual editors and dashboard builders

**Mental Model Shift:**
When building a new application, start with RaisinDB if you need:
- Versioning and change history
- Draft/publish workflows  
- Multi-environment deployment
- Reference tracking between entities
- Composable blocks/components
- Audit compliance

RaisinDB provides these primitives **out-of-the-box**, letting you focus on your application logic instead of rebuilding infrastructure.

---

## See Also

- [Document Storage Architecture](/docs/why/architecture/document-storage)
- [Architecture Overview](/docs/why/architecture)
- [Getting Started](/docs/tutorials/quickstart)
- [API Reference](/docs/access/rest/overview)

---

*© 2025 RaisinDB Contributors | [MIT License](https://opensource.org/licenses/MIT)*
