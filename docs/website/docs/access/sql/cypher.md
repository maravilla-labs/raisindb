# Cypher Graph Queries

RaisinDB supports **openCypher** graph pattern matching for querying relationships between nodes. This provides an expressive, visual way to navigate your content graph alongside the hierarchical tree structure.

## Why Cypher + Hierarchical Trees?

RaisinDB combines two powerful data models:

- **Hierarchical Paths** (`/content/blog/article`) - Natural for content organization, file systems, and taxonomies
- **Graph Relationships** (`:AUTHORED`, `:REFERENCES`, `:TAGS`) - Flexible connections across the hierarchy

This dual-model approach lets you:
- ✅ **Organize** content in intuitive folder-like paths
- ✅ **Connect** any nodes with typed relationships (authors, tags, references)
- ✅ **Query** using path operations (`PATH_STARTS_WITH`) or graph traversal (`MATCH`)
- ✅ **Index** both structures efficiently (RocksDB prefix scans + relationship indexes)

**Example:** Blog posts live in `/content/blog/2025/my-post` but can be **authored by** users, **tagged with** topics, and **reference** other articles - all without duplicating content.

## Basic Syntax

Cypher queries use the `cypher()` table-valued function within SQL:

```sql
SELECT * FROM cypher('
  MATCH (source)-[relationship]->(target)
  WHERE condition
  RETURN expressions
');
```

Cypher queries return individual columns for each item in the RETURN clause, with column names derived from the expressions (or aliases if specified).

## Supported Features

### ✅ Implemented

| Feature | Status | Example |
|---------|--------|---------|
| MATCH clause | ✅ Full | `MATCH (s)-[:TYPE]->(t)` |
| WHERE clause | ✅ Full | `WHERE s.id = "node-123"` |
| RETURN clause | ✅ Full | `RETURN s.id, t.workspace` |
| Relationship directions | ✅ Full | `->`, `<-`, `-` (bidirectional) |
| Relationship type filters | ✅ Full | `[:AUTHORED]`, `[:TAG\|CATEGORY]` |
| Property access | ✅ Full | `s.id`, `t.workspace` |
| Variable-length paths | ✅ Full | `*2`, `*1..3`, `*`, `*2..`, `*..5` |
| Aggregate functions | ✅ Full | `collect()`, `count()`, `sum()`, `avg()`, `min()`, `max()` |
| Grouping | ✅ Full | `RETURN c.name, collect(p.name)` |
| Graph algorithms | ✅ Comprehensive | `degree()`, `pageRank()`, `betweenness()`, `componentId()`, `communityId()` |
| Custom functions | ✅ Partial | `lookup(id, workspace)` |

### ❌ Not Yet Implemented

| Feature | Status | Alternative |
|---------|--------|-------------|
| CREATE | ❌ Planned | Use SQL INSERT + relationship API |
| DELETE | ❌ Planned | Use SQL DELETE |
| SET | ❌ Planned | Use SQL UPDATE |
| WITH clause | ❌ Planned | Chain multiple queries |
| UNWIND | ❌ Planned | Use SQL unnest |
| MERGE | ❌ Planned | Use SQL UPSERT |
| OPTIONAL MATCH | ❌ Planned | Use LEFT JOIN equivalent |

## Pattern Matching

### Basic Patterns

Match nodes connected by relationships:

```sql
-- Find all folder relationships
SELECT * FROM cypher('
  MATCH (s)-[:ntRaisinFolder]->(t)
  RETURN s.id, s.workspace, t.id, t.workspace
');
```

**Result structure:**
```json
{
  "result": {
    "s_id": "source-node-id",
    "s_workspace": "main",
    "t_id": "target-node-id",
    "t_workspace": "main"
  }
}
```

### Relationship Directions

Specify which way relationships flow:

```sql
-- Outgoing: find what this node points to
MATCH (source)-[:AUTHORED]->(target)
RETURN source.id, target.id

-- Incoming: find what points to this node
MATCH (source)<-[:AUTHORED]-(target)
RETURN source.id, target.id

-- Bidirectional: match either direction
MATCH (node1)-[:LINK]-(node2)
RETURN node1.id, node2.id
```

**Common patterns:**
- `->` : Outgoing (e.g., user AUTHORED article)
- `<-` : Incoming (e.g., article was AUTHORED BY user)
- `-` : Any direction (e.g., nodes are LINKED)

### Relationship Type Filters

Filter relationships by type:

```sql
-- Single type
MATCH (a)-[:AUTHORED]->(b)

-- Multiple types (OR logic)
MATCH (a)-[:TAG|CATEGORY]->(b)

-- Any type (no filter)
MATCH (a)-[r]->(b)
```

### Variable Binding

Capture relationships for later use:

```sql
-- Bind relationship to variable 'r'
SELECT * FROM cypher('
  MATCH (a)-[r:AUTHORED]->(b)
  RETURN a.id, r.type, b.id
');
```

### Variable-Length Paths

Match paths with multiple hops between nodes:

```sql
-- Friends of friends (exactly 2 hops)
SELECT * FROM cypher('
  MATCH (me:Person)-[:FRIEND*2]->(fof:Person)
  WHERE me.id = "user-123"
  RETURN fof.id, fof.workspace
');

-- Reachability within 1-3 hops
SELECT * FROM cypher('
  MATCH (start)-[:LINK*1..3]->(reachable)
  WHERE start.id = "node-1"
  RETURN DISTINCT reachable.id
');

-- Any length path (unbounded, max 10 hops by default)
SELECT * FROM cypher('
  MATCH (a)-[:KNOWS*]->(b)
  RETURN a.id, b.id
');

-- Track the path
SELECT * FROM cypher('
  MATCH (a)-[path:KNOWS*1..3]->(b)
  RETURN a.id, b.id, path.length AS hops
');
```

**Syntax patterns:**
- `*2` - Exactly 2 hops
- `*1..3` - Between 1 and 3 hops (inclusive)
- `*` - Unbounded (capped at 10 hops by default)
- `*2..` - Minimum 2 hops, no maximum (capped at 10)
- `*..5` - Maximum 5 hops, minimum 1

**Path properties:**
When you bind a variable-length relationship to a variable, you can access:
- `path.length` - Number of hops in the path
- `path.relationships` - Array of relationship objects in the path

**Performance characteristics:**
- ✅ DFS (Depth-First Search) algorithm for memory efficiency
- ✅ Automatic cycle detection prevents infinite loops
- ⚠️ Performance warning on depth > 5
- ⚠️ Path limit: 10,000 results maximum
- ⚠️ Default max depth: 10 for unbounded queries

**Best practices:**
- Always specify max depth when possible (`*1..3` instead of `*`)
- Use relationship type filters to reduce search space
- Add WHERE clauses to filter source/target nodes
- Use DISTINCT in RETURN to avoid duplicate paths

## Filtering with WHERE

Combine pattern matching with conditions:

```sql
-- Filter by node properties
SELECT * FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  WHERE user.id = "user-123"
  RETURN article.id, article.workspace
');

-- Multiple conditions
SELECT * FROM cypher('
  MATCH (s)-[:ntRaisinFolder]->(t)
  WHERE s.workspace = "main" AND t.workspace = "main"
  RETURN s.id AS source_id, t.id AS target_id
');
```

**Supported WHERE operators:**
- Equality: `=`, `!=`
- Comparison: `<`, `<=`, `>`, `>=`
- Logical: `AND`, `OR`, `NOT`

## RETURN Clause

### Property Access

Access node properties in RETURN:

```sql
-- Basic properties (always available)
MATCH (s)-[:LINK]->(t)
RETURN s.id, s.workspace, t.id, t.workspace

-- Lightweight: no storage fetch needed
-- These properties come from the relationship index
```

**Available properties (no fetch):**
- `node.id` - Node identifier
- `node.workspace` - Workspace name

**Note:** `path`, `type`, and `properties` require using `lookup()` function (see below).

### Aggregate Functions

Group and aggregate relationship data:

```sql
-- Count relationships per node
SELECT * FROM cypher('
  MATCH (s)-[:AUTHORED]->(t)
  RETURN s.id, count(t) AS article_count
');

-- Collect related nodes into arrays
SELECT * FROM cypher('
  MATCH (company)<-[:WORKS_AT]-(employee)
  RETURN company.id, collect(employee.id) AS employee_ids
');

-- Multiple aggregates
SELECT * FROM cypher('
  MATCH (category)<-[:IN_CATEGORY]-(product)
  RETURN
    category.id,
    count(product) AS total_products,
    collect(product.id) AS product_ids
');
```

**Supported aggregates:**
- `collect(expr)` - Collect values into array
- `count(expr)` - Count non-null values
- `sum(expr)` - Sum numeric values
- `avg(expr)` - Average of numeric values
- `min(expr)` - Minimum value
- `max(expr)` - Maximum value

**Grouping behavior:**
- Non-aggregate expressions become grouping keys
- Aggregates computed per group
- Example: `RETURN company.id, collect(employee.id)` groups by `company.id`

## Custom Functions

### lookup(id, workspace)

Fetch complete node data during graph traversal:

```sql
lookup(id, workspace) → node_object
```

**Parameters:**
- `id`: Node identifier (string)
- `workspace`: Workspace name (string)

**Returns JSONB object:**
```json
{
  "id": "node-id",
  "workspace": "main",
  "path": "/content/blog/article",
  "type": "my:Article",
  "properties": { "title": "Hello World", "status": "published" }
}
```

**Example - Fetch full node details:**

```sql
SELECT * FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  RETURN
    lookup(user.id, user.workspace) AS user_data,
    lookup(article.id, article.workspace) AS article_data
');
```

**Example - Access nested properties:**

```sql
SELECT
  user_data->>'properties'->>'name' AS author_name,
  article_data->>'properties'->>'title' AS article_title,
  article_data->>'path' AS article_path
FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  WHERE user.id = "user-123"
  RETURN
    lookup(user.id, user.workspace) AS user_data,
    lookup(article.id, article.workspace) AS article_data
');
```

**Performance considerations:**
- ✅ Use when you need full node data (path, type, properties)
- ✅ Good for small result sets (< 100 nodes)
- ❌ Avoid in large queries (each lookup = 1 storage fetch)
- 💡 Alternative: Use SQL JOIN with nodes table for better performance

### Graph Algorithm Functions

RaisinDB provides built-in graph algorithm functions for network analysis:

#### Degree Centrality

Calculate the number of relationships for a node:

```sql
-- Total degree (incoming + outgoing)
SELECT * FROM cypher('
  MATCH (node:Person)-[r:FRIEND]-(friend)
  RETURN
    node.id,
    degree(node) AS total_connections,
    collect(friend.id) AS friends
');

-- Incoming degree only
SELECT * FROM cypher('
  MATCH (article:Article)
  RETURN
    article.id,
    inDegree(article) AS incoming_links,
    outDegree(article) AS outgoing_links
');
```

**Available functions:**
- `degree(node)` - Total number of relationships (in + out)
- `inDegree(node)` - Number of incoming relationships
- `outDegree(node)` - Number of outgoing relationships

**Example - Find most connected nodes:**

```sql
SELECT
  node_id AS id,
  connections::integer AS connection_count
FROM cypher('
  MATCH (node:Person)
  RETURN node.id AS node_id, degree(node) AS connections
')
ORDER BY connection_count DESC
LIMIT 10;
```

**Example - Identify hubs and authorities:**

```sql
-- Hubs (many outgoing links)
SELECT * FROM cypher('
  MATCH (page:Page)
  WHERE outDegree(page) > 10
  RETURN page.id, outDegree(page) AS out_links
');

-- Authorities (many incoming links)
SELECT * FROM cypher('
  MATCH (page:Page)
  WHERE inDegree(page) > 20
  RETURN page.id, inDegree(page) AS in_links
');
```

**Use cases:**
- 🔍 **Find influencers** - Nodes with high degree are popular/important
- 🌐 **Detect hubs** - Nodes with many connections
- 📊 **Network analysis** - Understanding graph structure
- ⚖️ **Balance checking** - Compare in vs out degree

**Performance:**
- ✅ O(1) for outDegree - direct index lookup
- ⚠️ O(E) for inDegree - requires scanning (optimized with indexes)
- ✅ Efficient for individual nodes or small result sets

#### Shortest Path Functions

Find paths between nodes using BFS (Breadth-First Search) algorithms:

```sql
-- Find shortest path between two nodes
SELECT * FROM cypher('
  MATCH (start), (end)
  WHERE start.id = "node-A" AND end.id = "node-D"
  RETURN shortestPath(start, end) AS path
');

-- Find all shortest paths (all paths with minimum length)
SELECT * FROM cypher('
  MATCH (a), (b)
  WHERE a.id = "city-A" AND b.id = "city-B"
  RETURN allShortestPaths(a, b, 5) AS routes
');

-- Get distance only (shortest path length)
SELECT * FROM cypher('
  MATCH (me:Person), (other:Person)
  WHERE me.id = "user-1"
  RETURN
    other.id,
    distance(me, other) AS degrees_of_separation
  ORDER BY degrees_of_separation
');
```

**Available functions:**
- `shortestPath(startNode, endNode, maxDepth?)` - Returns shortest path object
- `allShortestPaths(startNode, endNode, maxDepth?)` - Returns array of all shortest paths
- `distance(startNode, endNode)` - Returns integer distance (number of hops)

**Path object structure:**
```json
{
  "nodes": [
    {"id": "node-A", "workspace": "main"},
    {"id": "node-C", "workspace": "main"},
    {"id": "node-D", "workspace": "main"}
  ],
  "relationships": [
    {"type": "LINK"},
    {"type": "LINK"}
  ],
  "length": 2
}
```

**Example - Friends of friends distance:**

```sql
SELECT
  result ->> 'person_id' AS id,
  (result ->> 'separation')::integer AS hops
FROM cypher('
  MATCH (me:Person), (other:Person)
  WHERE me.id = "user-1" AND other.id != "user-1"
  RETURN
    other.id AS person_id,
    distance(me, other) AS separation
')
WHERE (result ->> 'separation')::integer > 0
  AND (result ->> 'separation')::integer <= 3
ORDER BY hops
LIMIT 20;
```

**Example - Extract path details:**

```sql
SELECT
  result -> 'path' -> 'nodes' AS nodes,
  result -> 'path' -> 'relationships' AS relationships,
  (result -> 'path' ->> 'length')::integer AS path_length
FROM cypher('
  MATCH (a), (b)
  WHERE a.id = "start" AND b.id = "end"
  RETURN shortestPath(a, b, 10) AS path
');
```

**Use cases:**
- 🔍 **Degrees of separation** - How many hops between people
- 🗺️ **Route finding** - Shortest paths in networks
- 📊 **Reachability analysis** - What can be reached from a node
- 🔗 **Connection discovery** - How entities are connected

**Performance:**
- ✅ **O(V + E) BFS** - Optimal for unweighted graphs
- ⚠️ **Graph scan required** - Builds adjacency list from all relationships
- 💡 **Default maxDepth: 10** - Prevents unbounded searches
- 💡 **Recommended** - Use on graphs with < 100k relationships

**Return values:**
- `shortestPath()` returns empty object `{}` if no path exists
- `distance()` returns `-1` if no path exists
- `allShortestPaths()` returns empty array `[]` if no paths exist

### PageRank Algorithm

**Syntax:** `pageRank(node, dampingFactor?, maxIterations?)`

Calculates the PageRank score for a node, measuring its influence in the graph based on incoming relationships. Uses the power iteration algorithm with configurable damping factor (default: 0.85) and convergence criteria (default: 100 iterations).

**PageRank Formula:**
```
PR(v) = (1-d)/N + d * Σ(PR(u)/L(u))
```
Where:
- `d` = damping factor (0.85 = 85% follow links, 15% random jump)
- `N` = total nodes in graph
- `u` = nodes linking to v
- `L(u)` = outgoing links from u

**Example 1: Find Most Influential Nodes**

```sql
SELECT
  result ->> 'node_id' AS node_id,
  (result ->> 'pagerank')::float AS influence_score
FROM cypher('
  MATCH (n)
  RETURN n.id AS node_id, pageRank(n) AS pagerank
  ORDER BY pagerank DESC
  LIMIT 10
');
```

**Example 2: Custom Damping Factor**

```sql
-- Higher damping (0.95) = more weight on link structure
-- Lower damping (0.70) = more random jumps
SELECT
  result ->> 'node_id' AS node_id,
  (result ->> 'pagerank')::float AS score
FROM cypher('
  MATCH (n)
  RETURN n.id AS node_id, pageRank(n, 0.95) AS pagerank
');
```

**Example 3: Hub Detection in Citation Networks**

```sql
-- Find highly-cited papers in citation network
SELECT
  result ->> 'paper_id' AS paper_id,
  result ->> 'title' AS title,
  (result ->> 'citations')::integer AS citation_count,
  (result ->> 'pagerank')::float AS influence
FROM cypher('
  MATCH (paper)
  OPTIONAL MATCH (citing)-[:CITES]->(paper)
  WITH paper, count(citing) AS citations
  RETURN
    paper.id AS paper_id,
    paper.properties->>''title'' AS title,
    citations,
    pageRank(paper) AS pagerank
  ORDER BY pagerank DESC
  LIMIT 20
');
```

**Use Cases:**
- **Citation Networks**: Find most influential papers/authors
- **Social Networks**: Identify key influencers
- **Knowledge Graphs**: Rank important entities
- **Recommendation Systems**: Score item importance

### Closeness Centrality

**Syntax:** `closeness(node)`

Measures how close a node is to all other reachable nodes in the graph. A higher closeness score indicates the node can reach others with fewer hops, making it more "central" to the network.

**Closeness Formula:**
```
C(v) = (N-1) / Σ d(v, u)
```
Where:
- `N` = number of reachable nodes (including v)
- `d(v, u)` = shortest distance from v to u

Returns value between 0.0 (isolated) and 1.0 (perfectly central).

**Example 1: Find Central Hub Nodes**

```sql
SELECT
  result ->> 'node_id' AS node_id,
  (result ->> 'closeness')::float AS centrality_score
FROM cypher('
  MATCH (n)
  RETURN n.id AS node_id, closeness(n) AS closeness
  ORDER BY closeness DESC
  LIMIT 10
');
```

**Example 2: Compare Centrality Across Node Types**

```sql
SELECT
  result ->> 'node_type' AS node_type,
  (result ->> 'avg_closeness')::float AS avg_centrality
FROM cypher('
  MATCH (n)
  RETURN
    n.node_type AS node_type,
    avg(closeness(n)) AS avg_closeness
  GROUP BY node_type
  ORDER BY avg_closeness DESC
');
```

**Example 3: Find Communication Hubs**

```sql
-- Nodes with high closeness can broadcast information efficiently
SELECT
  result ->> 'user_id' AS user_id,
  result ->> 'username' AS username,
  (result ->> 'out_degree')::integer AS connections,
  (result ->> 'closeness')::float AS centrality
FROM cypher('
  MATCH (user)
  OPTIONAL MATCH (user)-[:FOLLOWS]->(other)
  WITH user, count(other) AS out_degree
  RETURN
    user.id AS user_id,
    user.properties->>''username'' AS username,
    out_degree AS connections,
    closeness(user) AS closeness
  WHERE closeness > 0.5
  ORDER BY closeness DESC
');
```

**Use Cases:**
- **Social Networks**: Find users with broad reach
- **Communication Networks**: Identify efficient broadcasters
- **Transportation Networks**: Locate central hubs
- **Organization Charts**: Find key coordinators

**Important Notes:**
- Closeness only considers **reachable** nodes (follows directed edges)
- Isolated nodes (no outgoing edges) return closeness = 0.0
- Works best on **connected components** of the graph
- In directed graphs, nodes with many outgoing edges typically have higher closeness

### Betweenness Centrality

**Syntax:** `betweenness(node)`

Measures how often a node appears on shortest paths between other nodes. Nodes with high betweenness are "bridges" that connect different parts of the graph and control information flow.

**Betweenness Formula:**
```
CB(v) = Σ(σst(v) / σst)
```
Where:
- `σst` = number of shortest paths from s to t
- `σst(v)` = number of those paths passing through v
- Normalized by `(n-1)(n-2)` for directed graphs

Returns value between 0.0 (never on shortest paths) and 1.0 (always on shortest paths).

**Example 1: Find Bridge Nodes**

```sql
SELECT
  result ->> 'node_id' AS node_id,
  (result ->> 'betweenness')::float AS bridge_score
FROM cypher('
  MATCH (n)
  RETURN n.id AS node_id, betweenness(n) AS betweenness
  ORDER BY betweenness DESC
  LIMIT 10
');
```

**Example 2: Identify Critical Infrastructure Nodes**

```sql
-- Find nodes whose removal would disconnect the network
SELECT
  result ->> 'server_id' AS server,
  result ->> 'location' AS location,
  (result ->> 'betweenness')::float AS criticality
FROM cypher('
  MATCH (server)
  WHERE server.node_type = ''Server''
  RETURN
    server.id AS server_id,
    server.properties->>''location'' AS location,
    betweenness(server) AS betweenness
  WHERE betweenness > 0.5
  ORDER BY betweenness DESC
');
```

**Example 3: Compare Bridge Scores by Department**

```sql
SELECT
  result ->> 'department' AS department,
  (result ->> 'avg_betweenness')::float AS avg_bridge_score
FROM cypher('
  MATCH (person)
  RETURN
    person.properties->>''department'' AS department,
    avg(betweenness(person)) AS avg_betweenness
  GROUP BY department
  ORDER BY avg_betweenness DESC
');
```

**Use Cases:**
- **Network Security**: Identify critical infrastructure nodes
- **Social Networks**: Find information brokers and influencers
- **Supply Chains**: Locate bottleneck points
- **Organization Analysis**: Find key connectors between teams
- **Transportation**: Identify critical junctions

**Algorithm Details:**
- Uses Brandes' algorithm - O(V*E) complexity
- More expensive than degree or closeness (requires BFS from every node)
- Best suited for graphs with < 10K nodes
- Bridge nodes often have high betweenness even if they have low degree

### Connected Components

**Syntax:** `componentId(node)`, `componentCount()`

Finds weakly connected components in the graph. A component is a maximal set of nodes where there exists a path between any two nodes (ignoring edge direction).

**Example 1: Identify Isolated Clusters**

```sql
SELECT
  result ->> 'node_id' AS node_id,
  (result ->> 'component')::integer AS cluster_id
FROM cypher('
  MATCH (n)
  RETURN n.id AS node_id, componentId(n) AS component
  ORDER BY component, node_id
');
```

**Example 2: Count Total Components**

```sql
SELECT
  (result ->> 'total_components')::integer AS num_clusters
FROM cypher('
  RETURN componentCount() AS total_components
');
```

**Example 3: Find Largest Component**

```sql
SELECT
  result ->> 'component_id' AS cluster,
  (result ->> 'size')::integer AS num_nodes
FROM cypher('
  MATCH (n)
  WITH componentId(n) AS component_id, count(n) AS size
  RETURN component_id, size
  ORDER BY size DESC
  LIMIT 1
');
```

**Example 4: Detect Disconnected Sub-Networks**

```sql
-- Find all nodes in the same component as a specific node
SELECT
  result ->> 'peer_id' AS peer_node
FROM cypher('
  MATCH (anchor), (peer)
  WHERE anchor.id = ''node-123''
    AND componentId(anchor) = componentId(peer)
  RETURN peer.id AS peer_id
');
```

**Use Cases:**
- **Network Topology**: Identify disconnected sub-networks
- **Data Quality**: Find isolated data islands
- **Social Analysis**: Discover separate social circles
- **Graph Validation**: Check if graph is fully connected
- **Cluster Analysis**: Group nodes into natural clusters

**Algorithm Details:**
- Uses BFS/DFS traversal - O(V + E) complexity
- Treats directed graph as undirected (weak connectivity)
- Component IDs are arbitrary integers (0, 1, 2, ...)
- Nodes in same component share same ID

### Community Detection (Label Propagation)

**Syntax:** `communityId(node)`, `communityCount()`

Detects communities using the Label Propagation Algorithm (LPA). Communities are groups of nodes with dense internal connections and sparse external connections. Unlike connected components, nodes within a community are more tightly connected than components require.

**Example 1: Detect Communities**

```sql
SELECT
  result ->> 'node_id' AS node_id,
  (result ->> 'community')::integer AS community_id
FROM cypher('
  MATCH (n)
  RETURN n.id AS node_id, communityId(n) AS community
  ORDER BY community, node_id
');
```

**Example 2: Count Communities**

```sql
SELECT
  (result ->> 'num_communities')::integer AS total_communities
FROM cypher('
  RETURN communityCount() AS num_communities
');
```

**Example 3: Find Community Sizes**

```sql
SELECT
  result ->> 'community_id' AS community,
  (result ->> 'members')::integer AS size
FROM cypher('
  MATCH (n)
  WITH communityId(n) AS community_id, count(n) AS members
  RETURN community_id, members
  ORDER BY members DESC
');
```

**Example 4: Analyze Inter-Community Connections**

```sql
-- Find bridges between communities
SELECT
  result ->> 'from_community' AS from_comm,
  result ->> 'to_community' AS to_comm,
  (result ->> 'connections')::integer AS edge_count
FROM cypher('
  MATCH (source)-[r]->(target)
  WHERE communityId(source) <> communityId(target)
  WITH communityId(source) AS from_community,
       communityId(target) AS to_community,
       count(r) AS connections
  RETURN from_community, to_community, connections
  ORDER BY connections DESC
');
```

**Example 5: Find Most Connected Community Members**

```sql
-- Within each community, find the most connected nodes
SELECT
  result ->> 'community_id' AS community,
  result ->> 'node_id' AS hub_node,
  (result ->> 'connections')::integer AS degree
FROM cypher('
  MATCH (n)-[r]->(other)
  WHERE communityId(n) = communityId(other)
  WITH communityId(n) AS community_id, n.id AS node_id, count(r) AS connections
  RETURN community_id, node_id, connections
  ORDER BY community_id, connections DESC
');
```

**Use Cases:**
- **Social Networks**: Discover friend groups and communities
- **Citation Networks**: Find research communities and topics
- **E-commerce**: Product recommendation clusters
- **Fraud Detection**: Identify suspicious activity patterns
- **Marketing**: Customer segmentation
- **Biology**: Protein interaction modules

**Algorithm Details:**
- Uses Label Propagation Algorithm (Raghavan et al. 2007)
- Time complexity: O(k * E) where k = iterations (typically < 100)
- Space complexity: O(V)
- Non-deterministic: may produce different valid communities each run
- Works on undirected view of graph
- Stops when labels converge or max iterations reached

**Important Notes:**
- **Communities vs Components**: Components are strict connectivity (path between any two nodes). Communities are dense regions with more internal than external connections.
- **Non-Determinism**: LPA may produce different valid communities on different runs (all valid)
- **Best For**: Large graphs where modularity-based methods are too slow
- **Convergence**: Usually converges in < 5 iterations for most real-world graphs

## Extracting Results

Cypher returns individual columns for each RETURN item. Column names are derived from expressions or aliases:

### Direct Column Access

```sql
-- Columns are created from RETURN expressions
SELECT
  s_id AS source_id,  -- From "s.id" in RETURN
  t_id AS target_id   -- From "t.id" in RETURN
FROM cypher('MATCH (s)-[:LINK]->(t) RETURN s.id, t.id');

-- Use aliases in RETURN to control column names
SELECT
  source_id,  -- Direct access to aliased column
  target_id   -- No JSON operators needed!
FROM cypher('MATCH (s)-[:LINK]->(t) RETURN s.id AS source_id, t.id AS target_id');
```

### Nested JSONB Data

```sql
-- For complex objects (like lookup() results), use arrow operators on the column
SELECT
  user_data->>'id' AS user_id,
  user_data->'properties'->>'email' AS user_email
FROM cypher('
  MATCH (u)-[:AUTHORED]->(a)
  RETURN lookup(u.id, u.workspace) AS user_data
');
```

### Type Casting

```sql
-- Cast column values to specific types
SELECT
  count::integer,
  distance::float
FROM cypher('MATCH (n) RETURN count(n) AS count, 1.5 AS distance');
```

## Practical Examples

### Example 1: Find All Articles by Author

```sql
SELECT
  result ->> 'article_id' AS id,
  result ->> 'article_workspace' AS workspace
FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  WHERE user.id = "user-alice"
  RETURN article.id AS article_id, article.workspace AS article_workspace
');
```

### Example 2: Count Relationships per Node

```sql
SELECT
  result ->> 'node_id' AS node_id,
  (result ->> 'relationship_count')::integer AS count
FROM cypher('
  MATCH (n)-[:LINK]->(other)
  RETURN n.id AS node_id, count(other) AS relationship_count
');
```

### Example 3: Collect Tags for Articles

```sql
SELECT
  result ->> 'article_id' AS article,
  result -> 'tags' AS tag_array
FROM cypher('
  MATCH (article)-[:TAGGED_WITH]->(tag)
  RETURN article.id AS article_id, collect(tag.id) AS tags
');
```

### Example 4: Find Mutual Connections

```sql
SELECT
  result ->> 'node1' AS node1,
  result ->> 'node2' AS node2,
  result ->> 'common_target' AS common
FROM cypher('
  MATCH (n1)-[:LINK]->(common)<-[:LINK]-(n2)
  WHERE n1.id < n2.id
  RETURN n1.id AS node1, n2.id AS node2, common.id AS common_target
');
```

### Example 5: Enrich with Full Node Data

```sql
SELECT
  result -> 'user' ->> 'id' AS user_id,
  result -> 'user' -> 'properties' ->> 'name' AS user_name,
  result -> 'article' -> 'properties' ->> 'title' AS article_title,
  result -> 'article' ->> 'path' AS article_path
FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  WHERE user.workspace = "main"
  RETURN
    lookup(user.id, user.workspace) AS user,
    lookup(article.id, article.workspace) AS article
')
LIMIT 10;
```

## Combining with SQL

Cypher integrates seamlessly with standard SQL:

### Use Cypher Results in WHERE

```sql
-- Find nodes that have relationships
SELECT n.*
FROM nodes n
WHERE n.id IN (
  SELECT result ->> 's_id'
  FROM cypher('MATCH (s)-[:LINK]->(t) RETURN s.id')
);
```

### Join with Nodes Table

```sql
-- Enrich graph results with full node data
SELECT
  n.id,
  n.path,
  n.properties ->> 'title' AS title,
  result ->> 'target_id' AS linked_to
FROM cypher('
  MATCH (s)-[:AUTHORED]->(t)
  RETURN s.id AS source_id, t.id AS target_id
') AS graph_data
JOIN nodes n ON n.id = graph_data.result ->> 'source_id';
```

### Combine Hierarchy + Graph

```sql
-- Find all blog articles and their authors
SELECT
  n.id,
  n.path,
  n.properties ->> 'title' AS title,
  result ->> 'author_id' AS author
FROM nodes n
CROSS JOIN LATERAL cypher(
  'MATCH (article)<-[:AUTHORED]-(author)
   WHERE article.id = "' || n.id || '"
   RETURN author.id AS author_id'
) AS authors
WHERE PATH_STARTS_WITH(n.path, '/content/blog/')
  AND n.node_type = 'my:Article';
```

## Performance Tips

1. **Filter Early** - Use WHERE to limit matched relationships before RETURN
2. **Minimize lookup()** - Only fetch full node data when needed
3. **Use Aggregates** - collect() is more efficient than multiple queries
4. **Index Relationships** - Relationship type filters use indexes
5. **Batch Queries** - Process multiple graph patterns in one query when possible
6. **Combine with PATH_STARTS_WITH** - Mix hierarchical and graph queries

## Common Patterns

### Pattern: Find all connections

```sql
MATCH (source)-[:RELATIONSHIP_TYPE]->(target)
RETURN source.id, target.id
```

### Pattern: Count by type

```sql
MATCH (s)-[r]->(t)
RETURN r.type, count(*) AS count
```

### Pattern: Group and collect

```sql
MATCH (parent)<-[:CHILD_OF]-(child)
RETURN parent.id, collect(child.id) AS children
```

### Pattern: Filter and fetch

```sql
MATCH (user)-[:AUTHORED]->(article)
WHERE user.workspace = "main"
RETURN lookup(article.id, article.workspace) AS article_data
```

## Migration from NEIGHBORS()

If you're using the SQL `NEIGHBORS()` function, Cypher provides a more expressive alternative:

**Old (SQL NEIGHBORS):**
```sql
SELECT n.id, n.name
FROM NEIGHBORS('node-123', 'OUT', 'AUTHORED') AS e
JOIN nodes n ON n.id = e.dst_id;
```

**New (Cypher):**
```sql
SELECT
  result ->> 'article_id' AS id,
  n.name
FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  WHERE user.id = "node-123"
  RETURN article.id AS article_id
') AS graph_data
JOIN nodes n ON n.id = graph_data.result ->> 'article_id';
```

**Benefits of Cypher:**
- More readable pattern syntax
- Built-in aggregation support
- Easier to express complex patterns
- Standard openCypher syntax

## Limitations

Current implementation limits:

- ❌ **No CREATE/DELETE** - Use SQL for mutations
- ❌ **No OPTIONAL MATCH** - All patterns must match
- ❌ **No WITH clause** - Cannot chain multiple MATCH patterns yet
- ❌ **Single workspace** - Cross-workspace queries limited (use lookup() manually)
- ⚠️ **Performance** - Large graph traversals may be slow (use indexes and filters)
- ⚠️ **Variable-length paths** - Limited to 10,000 results and 10 hops by default

## What's Next?

- [RaisinSQL Reference](raisinsql.md) - Complete SQL syntax
- [Query Examples](examples.md) - Real-world query patterns
- [Full-Text Search](fulltext.md) - Advanced search capabilities

## Related Documentation

- [openCypher Specification](https://opencypher.org/) - Official Cypher language reference
- [REST API Overview](../rest/overview.md) - HTTP endpoints for graph queries
