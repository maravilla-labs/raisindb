---
sidebar_position: 5
---

# Quick Start

:::info Coming Soon
The quick start guide is being prepared with hands-on examples. Check back soon for step-by-step tutorials!
:::

## What You'll Learn

This quick start guide will walk you through:

1. **Setting up RaisinDB** in under 5 minutes
2. **Creating your first repository** and workspace
3. **Defining a custom NodeType** for your data
4. **Creating and organizing nodes** in a tree structure
5. **Making your first commit** and exploring version history

## Prerequisites

- Basic understanding of REST APIs and JSON
- Familiarity with command-line tools or API clients (curl, Postman, etc.)
- RaisinDB instance running (see [Installation](/docs/getting-started/installation))

## Tutorial Outline

### Step 1: Repository Setup

Using the CLI:
```bash
# Create a new repository
raisin repo create my-blog

# Create a workspace
raisin workspace create my-blog main

# Verify the workspace
raisin workspace list my-blog
```

Or using the REST API:
```bash
# Create a workspace
curl -X PUT http://localhost:8080/api/workspaces/my-blog/main

# List workspaces
curl http://localhost:8080/api/workspaces/my-blog
```

### Step 2: Define Your First NodeType
```yaml
# blog-article.yaml
name: blog:Article
description: A blog post
properties:
  - name: title
    type: String
    required: true
  - name: content
    type: BlockContainer
    required: true
  - name: author
    type: Reference
allowed_children: ["raisin:Asset"]
versionable: true
publishable: true
```

Register via CLI:
```bash
raisin nodetype create my-blog main blog-article.yaml
```

Or via REST API:
```bash
curl -X POST http://localhost:8080/api/management/my-blog/main/nodetypes \
  -H "Content-Type: application/json" \
  -d @blog-article.yaml
```

### Step 3: Create Content Nodes

Using the CLI:
```bash
# Create your first article
raisin node create my-blog main /articles/welcome \
  --type blog:Article \
  --set title="My First Post" \
  --set content='{"uuid":"blk-1","block_type":"paragraph","content":{"text":"Welcome!"}}'

# List nodes
raisin node list my-blog main /articles
```

Or using the REST API:
```bash
curl -X POST http://localhost:8080/api/repository/my-blog/main/head/workspace/commit \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Add first article",
    "nodes": [{
      "path": "/articles/welcome",
      "node_type": "blog:Article",
      "properties": {
        "title": "My First Post",
        "content": {"uuid":"blk-1","block_type":"paragraph","content":{"text":"Welcome to RaisinDB!"}}
      }
    }]
  }'
```

### Step 4: Version Control Workflows

Using the CLI:
```bash
# Create a new branch for experiments
raisin branch create my-blog feature/new-layout

# Switch to the branch
raisin branch checkout my-blog feature/new-layout

# Make changes and commit
raisin commit my-blog main -m "Update article layout"

# Create a tag
raisin tag create my-blog v1.0 -m "First release"

# View commit history
raisin log my-blog main

# Rollback to a previous revision
raisin checkout my-blog main <revision-hash>
```

Or using the REST API:
```bash
# Create a branch
curl -X POST http://localhost:8080/api/management/repositories/tenant-id/my-blog/branches \
  -d '{"name": "feature/new-layout", "from": "main"}'

# Create a tag
curl -X POST http://localhost:8080/api/management/repositories/tenant-id/my-blog/tags \
  -d '{"name": "v1.0", "message": "First release"}'

# List commit history
curl http://localhost:8080/api/management/repositories/tenant-id/my-blog/revisions
```

### Step 5: Query Your Data

Using the CLI:
```bash
# Query nodes by type
raisin query my-blog main --type blog:Article

# Search with filters
raisin query my-blog main --filter 'title contains "First"'

# Get node by path
raisin node get my-blog main /articles/welcome

# Get audit trail
raisin audit my-blog main /articles/welcome
```

Or using the REST API:
```bash
# Query with JSON
curl -X POST http://localhost:8080/api/repository/my-blog/main/head/workspace/query \
  -d '{"node_type": "blog:Article", "limit": 10}'

# Get by path
curl http://localhost:8080/api/repository/my-blog/main/head/workspace/articles/welcome

# Audit trail
curl http://localhost:8080/api/audit/my-blog/main/workspace/articles/welcome
```

## What's Next?

After completing the quick start:

- 📚 [Dive deeper into concepts](/docs/why/concepts)
- 🏗️ [Understand the architecture](/docs/why/architecture)
- 📝 [Explore NodeType definitions](/docs/model/nodetypes/overview)
- 🔧 [Master the REST API](/docs/access/rest/overview)

## Example Use Cases

### Content Management System
- Create pages, articles, and media assets
- Organize content in folders and categories
- Version control for content changes
- Publishing workflows for content approval

### Product Catalog
- Define product, category, and variant NodeTypes
- Build hierarchical category structures
- Track inventory and pricing changes
- Support multiple product variants

### Knowledge Base
- Create documentation nodes with rich formatting
- Organize knowledge in topic hierarchies
- Track document revisions and updates
- Cross-reference related articles

## Need Help?

- 📖 [Read the full documentation](/docs/why/concepts)
- 🐛 [Report issues](https://github.com/maravilla-labs/raisindb/issues) on GitHub
- 💡 [Request features](https://github.com/maravilla-labs/raisindb/issues/new) or improvements
- 📧 Contact the maintainers for enterprise support