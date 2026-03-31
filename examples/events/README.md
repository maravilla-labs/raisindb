# Events App

An events management platform built on RaisinDB with a SvelteKit frontend.

## Project Structure

```
events/
├── package/                # RaisinDB content package
│   ├── manifest.yaml       # Package metadata
│   ├── workspaces/         # Workspace definitions
│   ├── nodetypes/          # Data schemas (Event, Venue, Speaker)
│   ├── archetypes/         # Page templates (EventPage, VenuePage, SpeakerPage)
│   ├── elementtypes/       # Content blocks (Hero, Text, SpeakerCard, Schedule)
│   └── content/            # Seed content
└── frontend/               # SvelteKit web application
    └── src/
        ├── lib/            # RaisinDB client, types, components
        └── routes/         # Pages (events, venues, speakers)
```

## Getting Started

### Prerequisites

- [RaisinDB](https://raisindb.com) server running locally
- `raisindb` CLI installed (`npm install -g @raisindb/cli`)
- Node.js 18+

### 1. Start your RaisinDB server

```bash
raisindb
```

### 2. Deploy the content package

```bash
cd package
raisindb package create --check .   # Validate
raisindb package create .            # Build
raisindb package upload events-0.1.0.rap
```

### 3. Run the frontend

```bash
cd frontend
npm install
npm run dev
```

Open http://localhost:5173

## Data Model

| Type | Description |
|------|-------------|
| `events:Event` | Events with dates, location, pricing, categories |
| `events:Venue` | Event venues with address, capacity |
| `events:Speaker` | Speakers with bio, social links |

## Frontend Routes

| Route | Description |
|-------|-------------|
| `/` | Home page with featured and upcoming events |
| `/events` | All events listing |
| `/events/:slug` | Event detail page |
| `/venues` | All venues listing |
| `/venues/:slug` | Venue detail page |
| `/speakers` | All speakers listing |
| `/speakers/:slug` | Speaker detail page |
