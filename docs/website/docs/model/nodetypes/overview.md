---
sidebar_position: 1
---

# NodeType Overview

NodeTypes are the schema definitions that determine the structure, validation, and behavior of nodes in RaisinDB. They are defined using YAML and provide strong typing for your data.

## What are NodeTypes?

NodeTypes serve as:
- **Schema definitions** - specify the structure of nodes
- **Validation rules** - ensure data integrity and consistency
- **Behavior specifications** - control versioning, publishing, and auditing
- **Relationship constraints** - define parent-child relationships

## YAML Structure

NodeTypes are defined using YAML with the following structure:

```yaml
name: namespace:TypeName
description: Human-readable description
icon: icon_name  # Optional UI icon
version: 1       # Schema version
strict: true     # Enforce strict validation
properties:
  - name: property_name
    type: PropertyType
    title: Display Name     # Optional
    description: Property description  # Optional
    required: true|false
    default: default_value  # Optional
allowed_children: ["namespace:ChildType"]  # Optional
versionable: true|false   # Enable version control
publishable: true|false   # Enable publishing workflow
auditable: true|false     # Enable audit logging
```

## Property Types

RaisinDB supports various property types:

### Basic Types
- `String` - Text values (UTF-8)
- `Number` - Numeric values (integers and floats)
- `Boolean` - True/false values
- `DateTime` - ISO 8601 datetime strings

### Text Types
- `Text` - Multi-line text content
- `Markdown` - Markdown-formatted text
- `Html` - HTML content

### Advanced Types
- `Resource` - File uploads and external resources
- `Reference` - References to other nodes
- `Json` - Arbitrary JSON objects
- `Array` - Arrays of other property types

### Example Property Definitions

```yaml
properties:
  # Basic string with validation
  - name: title
    type: String
    title: Article Title
    description: The main title of the article
    required: true

  # Optional number with default
  - name: view_count
    type: Number
    title: View Count
    description: Number of times this article was viewed
    required: false
    default: 0

  # Rich text content
  - name: content
    type: Markdown
    title: Article Content
    description: The main body of the article
    required: true

  # File resource
  - name: featured_image
    type: Resource
    title: Featured Image
    description: Main image for the article
    required: false

  # Reference to another node
  - name: author
    type: Reference
    title: Author
    description: Reference to the author node
    required: true
```

## Built-in NodeTypes

RaisinDB provides several built-in NodeTypes:

### raisin:Folder
```yaml
name: raisin:Folder
description: A folder to organize nodes
icon: folder_open
version: 2
properties:
  - name: description
    type: String
    title: Description
    description: A brief description of the folder
    required: false
allowed_children: []  # Can contain any node type
versionable: true
publishable: true
auditable: true
```

### raisin:Asset
```yaml
name: raisin:Asset
description: Media asset (image, video, document, etc.)
icon: image
version: 1
strict: true
properties:
  - name: title
    type: String
    required: true
  - name: file
    type: Resource
    required: true
  - name: file_type
    type: String
    required: false
  - name: file_size
    type: Number
    required: false
  - name: description
    type: String
    required: false
allowed_children: []
versionable: false
publishable: true
auditable: false
```

## Behavior Flags

### versionable
When `true`, nodes of this type:
- Track changes in commit history
- Can be reverted to previous versions
- Support branching and merging

### publishable
When `true`, nodes of this type:
- Can have draft and published states
- Support publishing workflows
- Can be scheduled for publication

### auditable
When `true`, nodes of this type:
- Log all changes for compliance
- Track who made changes and when
- Provide detailed audit trails

## Validation Rules

### strict
When `true`:
- Only defined properties are allowed
- Extra properties will cause validation errors
- Ensures strict adherence to schema

When `false`:
- Additional properties are allowed
- Provides flexibility for schema evolution
- Useful during development

### required
Properties marked as `required: true`:
- Must be provided when creating nodes
- Cannot be set to null or empty
- Will cause validation errors if missing

## Inheritance and Composition

### Allowed Children
The `allowed_children` array specifies which NodeTypes can be children:

```yaml
# Blog can contain articles and assets
name: blog:Blog
allowed_children: ["blog:Article", "raisin:Asset"]

# Article can contain assets and comments
name: blog:Article
allowed_children: ["raisin:Asset", "blog:Comment"]

# Comments cannot have children
name: blog:Comment
allowed_children: []
```

### Namespacing
Use namespaces to organize NodeTypes:
- `raisin:*` - Built-in system types
- `blog:*` - Blog-related types
- `ecommerce:*` - E-commerce types
- `cms:*` - Content management types

## Example: Blog Article NodeType

```yaml
name: blog:Article
description: A blog post or article
icon: article
version: 1
strict: true

properties:
  - name: title
    type: String
    title: Article Title
    description: The main title of the blog post
    required: true

  - name: slug
    type: String
    title: URL Slug
    description: URL-friendly version of the title
    required: true

  - name: content
    type: Markdown
    title: Article Content
    description: The main body content in Markdown
    required: true

  - name: excerpt
    type: Text
    title: Excerpt
    description: Brief summary of the article
    required: false

  - name: published_date
    type: DateTime
    title: Published Date
    description: When this article was published
    required: false

  - name: author
    type: Reference
    title: Author
    description: Reference to the author profile
    required: true

  - name: tags
    type: Array
    title: Tags
    description: Array of tag strings
    required: false

  - name: featured_image
    type: Resource
    title: Featured Image
    description: Main image for the article
    required: false

allowed_children: ["raisin:Asset", "blog:Comment"]
versionable: true
publishable: true
auditable: true
```

## Registering NodeTypes

NodeTypes are registered via the REST API:

```bash
POST /api/nodetypes/{repo}
Content-Type: application/json

{
  "nodetype_yaml": "name: blog:Article\ndescription: ..."
}
```

## Next Steps

- 📖 [Learn core concepts](/docs/why/concepts)
- 🏗️ [Understand the architecture](/docs/why/architecture)
- 🔧 [Explore the REST API](/docs/access/rest/overview)