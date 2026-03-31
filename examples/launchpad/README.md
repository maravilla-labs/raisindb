# Launchpad

Minimal demo app for testing RaisinDB + raisin-client-js end-to-end.

## Architecture

```
RaisinDB                              Frontend
────────────────────────────────────────────────────────
launchpad:Page (NodeType)             (base hierarchy)
    ↓
launchpad:LandingPage (Archetype)  →  LandingPage.svelte
    ↓ properties.content
launchpad:Hero (ElementType)       →  Hero.svelte
launchpad:TextBlock (ElementType)  →  TextBlock.svelte
launchpad:FeatureGrid (ElementType)→  FeatureGrid.svelte
```

## Two Mappings

1. **Archetype → Page component** (layout/template for page types)
2. **ElementType → Element component** (content blocks)

## Structure

```
launchpad/
├── package/                    # RaisinDB content package
│   ├── manifest.yaml
│   ├── nodetypes/
│   │   └── page.yaml
│   ├── archetypes/
│   │   └── landing-page.yaml
│   ├── elementtypes/
│   │   ├── hero.yaml
│   │   ├── text-block.yaml
│   │   └── feature-grid.yaml
│   ├── content/
│   │   └── launchpad/
│   │       ├── home/.node.yaml
│   │       ├── about/.node.yaml
│   │       └── contact/.node.yaml
│   └── workspaces/
│       └── launchpad.yaml
│
└── frontend/                   # SvelteKit SPA
    └── src/
        ├── lib/
        │   ├── raisin.ts
        │   └── components/
        │       ├── pages/
        │       │   └── LandingPage.svelte
        │       └── elements/
        │           ├── Hero.svelte
        │           ├── TextBlock.svelte
        │           └── FeatureGrid.svelte
        └── routes/
            └── [...slug]/
```

## Getting Started

### 1. Start RaisinDB

```bash
# From raisindb root
cargo run --bin raisin-server
```

The server should be running on `localhost:8081`.

### 2. Install the Launchpad Package

```bash
# Install the package to RaisinDB
raisindb package install examples/launchpad/package
```

### 3. Start the Frontend

```bash
cd examples/launchpad/frontend
npm install
npm run dev
```

Open http://localhost:5173 in your browser.

## How It Works

1. **SvelteKit SPA** connects to RaisinDB via `@raisindb/client` WebSocket
2. **Navigation** loads root children from the workspace
3. **Page route** fetches page by path using `nodes.getByPath()`
4. **Archetype** determines which page layout component to use
5. **Elements** in `properties.content` are rendered using the element component mapping

## Testing Checklist

- [ ] RaisinDB server running on localhost:8081
- [ ] Package installed successfully
- [ ] Frontend connects to RaisinDB
- [ ] Navigation loads (home, about, contact)
- [ ] Page content renders correctly
- [ ] Hero sections display
- [ ] Text blocks display
- [ ] Feature grids display

## Next Steps

- Add authentication (raisin-auth integration)
- Add flows and triggers
- Add contact form submission
