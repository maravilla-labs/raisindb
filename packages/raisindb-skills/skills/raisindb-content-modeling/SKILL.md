---
name: raisindb-content-modeling
description: "Define NodeTypes, Archetypes, and ElementTypes for your RaisinDB package. Use when creating data schemas, page templates, or content blocks."
---

# RaisinDB Content Modeling

Define content schemas, page templates, and composable blocks using YAML files inside a RaisinDB package.

**MANDATORY**: After creating or modifying ANY `.yaml` or `.node.yaml` file in `package/`, immediately run:

    npm run validate

Fix all errors before proceeding. Never skip validation.

## 1. NodeType YAML

NodeTypes define the data schema for content. Place files in `package/nodetypes/`.

### Naming

Use `namespace:PascalCase` format: `myapp:Article`, `launchpad:Page`, `shop:Product`.

### Full Example

```yaml
name: launchpad:Page
title: Page
description: Base page type for Launchpad content
icon: file-text
color: "#6366f1"
version: 1

properties:
  - name: title
    title: Title
    type: String
    required: true
    is_translatable: true
    index:
      - Fulltext
  - name: slug
    title: Slug
    type: String
    required: true
  - name: description
    title: Description
    type: String
    required: false
    is_translatable: true

versionable: true
publishable: true
auditable: true
indexable: true
```

### Property Types

| Type | Stored as | Example |
|------|-----------|---------|
| `String` | JSON string | `"hello"` |
| `Number` | JSON number | `42` |
| `Boolean` | JSON boolean | `true` |
| `Date` | ISO 8601 string | `"2025-01-15"` |
| `Object` | JSON object | `{"key": "val"}` |
| `Array` | JSON array | `["a", "b"]` |
| `Reference` | `raisin:ref` object | See below |
| `Resource` | Resource metadata | File attachment |

### Property Options

| Option | Type | Description |
|--------|------|-------------|
| `required` | boolean | Must be set on creation |
| `is_translatable` | boolean | Enable i18n translation |
| `default` | any | Default value when not provided |
| `index` | string[] | Index types, e.g. `[Fulltext]` |

### Top-Level Flags

| Flag | Effect |
|------|--------|
| `versionable` | Tracks revision history, enables draft/publish workflow |
| `publishable` | Adds publish/unpublish lifecycle |
| `auditable` | Records who changed what and when |
| `indexable` | Includes in full-text search index |

### Reference Values at Runtime

A `Reference` property stores a `raisin:ref` object: `{"raisin:ref": "node-id-or-path", "raisin:workspace": "workspace-name"}`.

---

## 2. Archetype YAML

Archetypes define page templates linking a NodeType to editor fields. Place files in `package/archetypes/`. Multiple archetypes can share one NodeType for different layouts.

### Example

```yaml
name: launchpad:LandingPage
title: Landing Page
description: Landing page template with hero, content blocks, and features
icon: layout
color: "#6366f1"
base_node_type: launchpad:Page
version: 1

fields:
  - $type: TextField
    name: title
    title: Page Title
    required: true
    translatable: true

  - $type: TextField
    name: slug
    title: URL Slug
    required: true

  - $type: SectionField
    name: content
    title: Page Content
    allowed_element_types:
      - launchpad:Hero
      - launchpad:TextBlock
      - launchpad:FeatureGrid

publishable: true
```

### Field Types

| Field Type | Purpose |
|------------|---------|
| `TextField` | Text input (single or multi-line) |
| `NumberField` | Numeric input |
| `BooleanField` | Toggle / checkbox |
| `DateField` | Date or datetime picker |
| `MediaField` | File/image upload (maps to `Resource` property) |
| `RichTextField` | Rich text / HTML editor |
| `SectionField` | Container for ElementTypes -- the composition mechanism |
| `CompositeField` | Group of sub-fields, optionally repeatable |

### Field Options

All fields support: `name`, `title`, `required`, `translatable`, `description`.

### SectionField -- Composition Mechanism

`SectionField` composes pages from ElementTypes. Declare which element types are allowed via `allowed_element_types`. At runtime the property stores an array of element objects.

### CompositeField -- Repeatable Sub-Fields

Use `CompositeField` with `repeatable: true` for arrays of structured objects. You can nest `SectionField` inside a `CompositeField`:

```yaml
  - $type: CompositeField
    name: columns
    title: Columns
    repeatable: true
    translatable: true
    fields:
      - $type: TextField
        name: id
        title: Column ID
        required: true

      - $type: TextField
        name: title
        title: Column Title
        required: true
        translatable: true

      - $type: SectionField
        name: cards
        title: Cards
        translatable: true
        allowed_element_types:
          - launchpad:KanbanCard
```

---

## 3. ElementType YAML

ElementTypes are composable content blocks placed inside `SectionField` containers. Place files in `package/elementtypes/`. They use the same field types as archetypes.

### Hero Example

```yaml
name: launchpad:Hero
title: Hero Section
description: Full-width hero section with headline, subheadline, and call-to-action
icon: image
color: "#8b5cf6"
version: 1

fields:
  - $type: TextField
    name: headline
    title: Headline
    required: true
    translatable: true

  - $type: TextField
    name: subheadline
    title: Subheadline
    required: false
    translatable: true

  - $type: TextField
    name: cta_text
    title: CTA Button Text
    required: false
    translatable: true

  - $type: TextField
    name: cta_link
    title: CTA Button Link
    required: false

  - $type: MediaField
    name: background_image
    title: Background Image
    required: false
```

### RichTextField Example

Use `RichTextField` for rich text / HTML content:

```yaml
  - $type: RichTextField
    name: content
    title: Content
    required: true
    translatable: true
```

### ElementType with Nested CompositeField

ElementTypes can use `CompositeField` with `repeatable: true` for arrays of structured items:

```yaml
name: launchpad:FeatureGrid
title: Feature Grid
icon: grid-3x3
color: "#f59e0b"
version: 1

fields:
  - $type: TextField
    name: heading
    title: Section Heading
    translatable: true

  - $type: CompositeField
    name: features
    title: Features
    repeatable: true
    fields:
      - $type: TextField
        name: title
        title: Feature Title
        required: true
        translatable: true
      - $type: TextField
        name: description
        title: Feature Description
        required: true
        translatable: true
```

### Runtime Representation

Elements appear in `properties.content[]` with `uuid` and `element_type`:

```json
{ "uuid": "hero-1", "element_type": "launchpad:Hero", "headline": "Launch Your Vision" }
```

---

## 4. Mixins

Mixins are reusable property sets. Define them as NodeTypes with `is_mixin: true`. Place files in `package/mixins/`.

```yaml
name: myapp:SEOFields
title: SEO Fields
description: Common SEO properties for pages
is_mixin: true

properties:
  - name: meta_title
    title: Meta Title
    type: String
  - name: meta_description
    title: Meta Description
    type: String
  - name: og_image
    title: Open Graph Image
    type: Resource
```

Register under `provides.mixins` in `manifest.yaml`. Mixins install before NodeTypes.

---

## 5. Workspace YAML

Workspaces scope content and control allowed NodeTypes. Place files in `package/workspaces/`.

```yaml
name: launchpad
title: Launchpad
description: Content workspace for Launchpad portal
icon: rocket
color: "#6366f1"

allowed_node_types:
  - launchpad:Page
  - raisin:Folder
  - raisin:Asset        # Required for file uploads

allowed_root_node_types:
  - raisin:Folder
  - launchpad:Page

root_structure:
  - name: pages
    node_type: raisin:Folder
    title: Pages
    description: Site pages
```

- `allowed_node_types` -- types that can be created in this workspace
- `allowed_root_node_types` -- types allowed at root level
- `root_structure` -- nodes created automatically on workspace init

---

## 6. manifest.yaml

Declares package metadata and all provided types. Place at `package/manifest.yaml`.

```yaml
name: launchpad-next
version: 1.0.5
title: Launchpad Next
description: LaunchKit customer portal
author: SOLUTAS GmbH

provides:
  nodetypes:
    - launchpad:Page
  archetypes:
    - launchpad:LandingPage
    - launchpad:KanbanBoard
    - launchpad:FileBrowser
  elementtypes:
    - launchpad:Hero
    - launchpad:TextBlock
    - launchpad:FeatureGrid
    - launchpad:KanbanCard
  workspaces:
    - launchpad
  functions:
    - /lib/launchpad/handle-read-receipt
  triggers:
    - /triggers/on-read-receipt

workspace_patches:
  launchpad:
    allowed_node_types:
      add:
        - launchpad:Page
        - raisin:Folder
        - raisin:Asset          # Required for file uploads
```

Supported `provides` keys: `nodetypes`, `archetypes`, `elementtypes`, `mixins`, `workspaces`, `functions`, `triggers`, `flows`. Each maps to its directory (`nodetypes/`, `archetypes/`, etc.; functions use `content/functions/lib/`, triggers use `content/functions/triggers/`).

Use `workspace_patches` to add allowed node types to existing workspaces without overwriting their definition. Always include `raisin:Asset` if the workspace needs file uploads — without it, uploads will fail silently.

---

## 7. Content YAML

Content files define initial data installed with the package. Place `.node.yaml` files inside `package/content/{workspace}/{path}/`. The directory structure maps to the node tree.

File: `package/content/launchpad/launchpad/home/.node.yaml`

```yaml
node_type: launchpad:Page
archetype: launchpad:LandingPage
properties:
  title: Welcome to Launchpad
  slug: home
  description: Your gateway to launching amazing projects
  content:
    - uuid: hero-1
      element_type: launchpad:Hero
      headline: Launch Your Vision
      subheadline: Build, deploy, and scale your ideas with Launchpad
      cta_text: Get Started
      cta_link: /contact

    - uuid: intro-1
      element_type: launchpad:TextBlock
      heading: Why Launchpad?
      content: |
        Launchpad is your all-in-one platform for turning ideas into reality.

    - uuid: features-1
      element_type: launchpad:FeatureGrid
      heading: Features
      features:
        - icon: zap
          title: Fast Development
          description: Build and iterate quickly with our modern stack
```

### Content YAML Rules

- `node_type` -- required, must match a type in `provides.nodetypes`
- `archetype` -- optional, links to an archetype for the editor UI
- `properties` -- the node's property values
- Each element in `content` needs a unique `uuid` and a valid `element_type`
- Element fields correspond to the ElementType's field definitions

---

## 8. Validation

**MANDATORY** — run after every YAML change:

```bash
npm run validate
```

Do NOT proceed until all errors are resolved.

### Common Errors

| Error | Cause |
|-------|-------|
| `INVALID_NODE_TYPE_NAME` | Use `namespace:PascalCase` format (e.g. `myapp:Article`) |
| `MISSING_REQUIRED_FIELD` | Manifest needs `name` and `version` at minimum |
| `UNKNOWN_NODE_TYPE_REFERENCE` | Referenced type not listed in `provides.nodetypes` |
| `DUPLICATE_PROPERTY` | No duplicate property names within a single NodeType |
| `UNKNOWN_ELEMENT_TYPE` | Element type in `allowed_element_types` not in `provides.elementtypes` |
| `MISSING_BASE_NODE_TYPE` | Archetype `base_node_type` not found in `provides.nodetypes` |

Every type in `provides` must have a corresponding YAML file in the matching directory (`nodetypes/`, `archetypes/`, `elementtypes/`, `mixins/`, `workspaces/`). File names use kebab-case (e.g., `landing-page.yaml` for `launchpad:LandingPage`).
