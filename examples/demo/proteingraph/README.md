# ProteinGraph Demo: Alzheimer's Drug Discovery

A bioinformatics demo showcasing RaisinDB's graph capabilities for drug discovery and protein interaction network analysis.

## What This Demo Shows

| Feature | What It Does | Bioinformatics Use |
|---------|--------------|-------------------|
| **GRAPH_TABLE** | ISO SQL:2023 graph queries | Protein interaction networks |
| **Variable-length paths** | Multi-hop traversals (`*1..3`) | Pathway discovery |
| **PostgreSQL compatible** | Works with psql, Jupyter, DBeaver | Use your existing tools |
| **Vector embeddings** | Similarity search | Protein homolog finding |

## Quick Start

```bash
# 1. Create repository 'proteingraph' in admin console (http://localhost:3000)
#    and get your API key from Settings > API Keys

# 2. Set your API key
export RAISIN_API_KEY="raisin_YOUR_API_KEY_HERE"

# 3. Load the demo data
psql "postgresql://default:${RAISIN_API_KEY}@localhost:5432/proteingraph" -f setup-demo.sql

# 4. Run the demo queries
psql "postgresql://default:${RAISIN_API_KEY}@localhost:5432/proteingraph" -f demo-queries.sql

# 5. (Optional) Open Jupyter notebook for visualization
pip install psycopg2-binary pandas networkx matplotlib
jupyter notebook jupyter-demo.ipynb
```

## Prerequisites

1. **RaisinDB** running locally (default: `localhost:5432` for pgwire, `localhost:8080` for HTTP)
2. **psql** or **DBeaver** for SQL queries
3. **Python 3.8+** for Jupyter demo (optional)

## Setup

### 1. Start RaisinDB

```bash
# From the raisindb root directory
cargo run --release
```

### 2. Create a Repository and Get API Key

You need to create a repository first and obtain an API key. You can do this via:

**Option A: Admin Console (recommended)**
1. Open the admin console at `http://localhost:3000`
2. Create a new repository named `proteingraph`
3. Go to Settings > API Keys to create/copy your API key

**Option B: HTTP API**
```bash
# Create repository (replace YOUR_ADMIN_TOKEN with your admin token)
curl -X POST http://localhost:8080/api/repositories \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "proteingraph", "description": "Bioinformatics demo"}'
```

### 3. Connection String Format

The PostgreSQL connection string format is:
```
postgresql://{tenant}:{api_key}@{host}:{port}/{repository}
```

Example:
```
postgresql://default:raisin_XXXXXXXXXXXX@localhost:5432/proteingraph
```

| Component | Description | Example |
|-----------|-------------|---------|
| `tenant` | Tenant ID (usually `default`) | `default` |
| `api_key` | Your RaisinDB API key | `raisin_N7U7POgxOh9W...` |
| `host` | Server hostname | `localhost` |
| `port` | pgwire port | `5432` |
| `repository` | Repository name | `proteingraph` |

### 4. Load the Demo Data

```bash
# Replace with your actual API key
export RAISIN_API_KEY="raisin_YOUR_API_KEY_HERE"

# Load the demo data
psql "postgresql://default:${RAISIN_API_KEY}@localhost:5432/proteingraph" -f setup-demo.sql
```

Or run individual statements in DBeaver/DataGrip.

### 5. Run Demo Queries

#### Option A: psql
```bash
psql "postgresql://default:${RAISIN_API_KEY}@localhost:5432/proteingraph" -f demo-queries.sql
```

#### Option B: Jupyter Notebook
```bash
pip install psycopg2-binary pandas networkx matplotlib
jupyter notebook jupyter-demo.ipynb
```

Update the connection string in the notebook with your API key.

#### Option C: DBeaver
1. Create new PostgreSQL connection
2. Host: `localhost`, Port: `5432`, Database: `proteingraph`
3. Username: `default`, Password: `YOUR_API_KEY`
4. Open `demo-queries.sql` and run queries

## Demo Flow (5-10 min lightning talk)

| Time | What to Show | Hook |
|------|--------------|------|
| 0:00-0:30 | Intro | "What if your SQL could do graph queries natively?" |
| 0:30-2:30 | **WOW #1** | Run GRAPH_TABLE query - "No Neo4j. No Cypher. Just SQL." |
| 2:30-4:00 | **WOW #2** | Variable-length paths - "Find proteins 2-3 hops away" |
| 4:00-5:30 | **WOW #3** | Same query in Jupyter - "Works with your tools" |
| 5:30-6:00 | Close | "PostgreSQL compatible, graph-native, vector-ready" |

## Key Queries

### WOW #1: SQL that speaks Graph
```sql
-- Find drug targets 2-3 hops from Aducanumab
SELECT * FROM GRAPH_TABLE(
    MATCH (drug:Drug)-[:TARGETS]->(t:Protein)-[:INTERACTS_WITH*1..3]->(downstream:Protein)
    WHERE drug.name = 'Aducanumab'
    COLUMNS (drug.name, t.properties->>'gene_id' AS target, downstream.properties->>'gene_id')
)
```

### WOW #2: Variable-Length Paths
```sql
-- Proteins 2-3 hops from APP
SELECT * FROM GRAPH_TABLE(
    MATCH (start:Protein)-[:INTERACTS_WITH*2..3]->(distant:Protein)
    WHERE start.path = '/alzheimer-study/proteins/APP'
    COLUMNS (start.properties->>'gene_id' AS source, distant.properties->>'gene_id' AS target)
) LIMIT 20
```

### WOW #3: PostgreSQL Compatibility
```python
# Same query in Jupyter
import psycopg2
import pandas as pd

conn = psycopg2.connect("postgresql://default:API_KEY@localhost:5432/default")
df = pd.read_sql(query, conn)
```

## Data Model

```
/alzheimer-study/
├── /proteins/     (25 proteins: APP, PSEN1, BACE1, APOE, MAPT, etc.)
├── /drugs/        (6 drugs: Aducanumab, Lecanemab, Donepezil, etc.)
└── /diseases/     (1 disease: Alzheimer)

Relationships:
├── INTERACTS_WITH  (~40 protein-protein interactions from STRING)
├── TARGETS         (6 drug-target relationships)
└── ASSOCIATED_WITH (11 disease associations from DisGeNET)
```

## Files

| File | Description |
|------|-------------|
| `setup-demo.sql` | Creates schema, nodes, and relationships |
| `demo-queries.sql` | The 3 wow moment queries + bonus queries |
| `jupyter-demo.ipynb` | Interactive notebook with network visualization |
| `data/alzheimer_proteins.json` | Source data (50 proteins, 500 interactions) |

## Data Sources

- **STRING Database** - Protein-protein interactions (CC-BY)
- **DrugBank** - Drug-target relationships
- **DisGeNET** - Gene-disease associations (CC-BY-NC)
- **UniProt** - Protein metadata (CC-BY)

## Talking Points

1. **"SQL + Graph in one query"** - ISO SQL:2023 GRAPH_TABLE, no Cypher needed
2. **"Variable-length paths"** - `*1..3` for multi-hop traversals in SQL
3. **"Works with your tools"** - psql, DBeaver, Jupyter, R - no new clients
4. **"Graph algorithms"** - PageRank, Louvain, centrality built-in
5. **"Vector + Graph"** - Embeddings and relationships in one DB

## Requirements

- RaisinDB running locally
- psql or DBeaver for SQL queries
- Python 3.8+ for Jupyter demo:
  - psycopg2-binary
  - pandas
  - networkx
  - matplotlib
