# Common Schema Patterns

## Page Type

A page with a URL, title, and rich content body. Good for articles, blog posts,
landing pages, and documentation.

```yaml
properties:
  - name: title
    type: String
    title: Title
    required: true
    indexed: true

  - name: slug
    type: String
    title: URL Slug
    required: true
    indexed: true

  - name: description
    type: String
    title: Description

  - name: content
    type: Section
    title: Content
```

## Data Entity

A structured record without a page-like URL. Good for products, events, settings,
or any domain object.

```yaml
properties:
  - name: name
    type: String
    title: Name
    required: true
    indexed: true

  - name: status
    type: String
    title: Status
    indexed: true

  - name: properties
    type: Json
    title: Properties
```

## User Content

Content submitted by users (comments, reviews, profiles). Typically has an author
reference and timestamps.

```yaml
properties:
  - name: author
    type: Reference
    title: Author
    indexed: true

  - name: body
    type: String
    title: Body
    required: true

  - name: createdAt
    type: DateTime
    title: Created At
    indexed: true

  - name: approved
    type: Boolean
    title: Approved
    indexed: true
```

## Asset Node

A reference to a binary asset (image, document, video). The binary data is stored
separately; the node holds metadata.

```yaml
properties:
  - name: title
    type: String
    title: Title
    indexed: true

  - name: filename
    type: String
    title: Filename

  - name: mimeType
    type: String
    title: MIME Type
    indexed: true

  - name: size
    type: Int
    title: File Size (bytes)

  - name: alt
    type: String
    title: Alt Text
```

## Choosing Property Types

| Need | Type | Notes |
|------|------|-------|
| Short text | `String` | Titles, names, slugs |
| Long text | `String` | RaisinDB String handles any length |
| Number | `Int` or `Float` | Use Int for counts, Float for measurements |
| Yes/No | `Boolean` | Flags, toggles |
| Date/Time | `DateTime` | ISO 8601 format |
| Link to another node | `Reference` | Creates a graph edge |
| Rich content area | `Section` | Container for ElementType blocks |
| Multiple values | `StringList` | Tags, categories |
| Multiple links | `ReferenceList` | Related items |
| Flexible data | `Json` | Unstructured or dynamic data |
