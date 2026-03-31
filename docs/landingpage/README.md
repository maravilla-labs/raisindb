# RaisinDB Landing Page

A "Coming Soon" landing page for RaisinDB, showcasing the core features without documentation or API reference links.

## Overview

This is a standalone Docusaurus site based on `docs/website` but simplified for a pre-launch landing page. It highlights RaisinDB's technical features without linking to documentation or API references.

## Key Differences from Main Website

- No documentation sidebar or pages
- No API reference links
- "Coming Soon" message in hero section
- Focus on technical features and architecture
- Simplified navigation (GitHub link only)
- Simplified footer (Community links only)

## Running Locally

```bash
cd docs/landingpage
npm install
npm start
```

The site will be available at `http://localhost:3000`

## Building for Production

```bash
npm run build
npm run serve
```

## Features Highlighted

- RaisinSQL (PostgreSQL-compatible queries)
- Git-like branching and revisions
- Copy/move operations (copy_tree, move_node)
- Vector embeddings and KNN search
- Graph relationships with NEIGHBORS()
- Full-text search (Tantivy)
- NodeType schemas
- Translation support
- RocksDB backend

## License

Copyright © 2025 Maravilla Labs. Licensed under Business Source License 1.1.
