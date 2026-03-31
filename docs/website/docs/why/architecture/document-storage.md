# Document Storage Architecture

**Last Updated**: October 21, 2025

## Overview

RaisinDB implements a Git-like versioning system for content management, storing data in a document-oriented format with immutable snapshots and branch isolation. The system uses **RocksDB** as the primary storage backend with a fragmented index architecture for high-performance operations.

The document storage leverages **content-addressed trees**, **revision snapshots**, and **natural ordering** to provide efficient time-travel queries, branch isolation, and scalable hierarchical operations.

---

## Architecture Highlights

### Production-Ready Features

- **Immutable Node Snapshots**: Every revision creates immutable snapshots of changed nodes
- **Content-Addressed Trees**: Directory structures stored as Merkle trees
- **Branch Isolation**: True Git-like branching with independent workspace deltas
- **Structural Sharing**: Unchanged subtrees are reused across revisions (10-20x faster commits)
- **Natural Ordering**: Fractional indexing enables O(1) child reordering without cascading updates
- **Fragmented Indexes**: 16 specialized column families for different query patterns

### Backend Implementations

RaisinDB supports multiple storage backends through a unified `Storage` trait:

- **RocksStorage**: Production-ready persistent storage using RocksDB with advanced indexing
- **InMemoryStorage**: Fast in-memory storage for testing and development
- **Future**: PostgreSQL and MySQL support planned

---

## Content Model

Repository data is organized into several key collections:

### Core Collections

- **nodes**: Immutable node documents (workspace-scoped, repository-shared)
- **revisions**: Revision metadata and snapshot references
- **trees**: Content-addressed directory trees (Merkle trees)
- **branches**: Branch metadata with HEAD pointers
- **nodetypes**: Schema definitions (repository-scoped, shared across branches)
- **ordered_children**: Natural ordering with fractional indexes (separate from node data)

### Auxiliary Collections

- **workspace_deltas**: Draft changes before commit (branch-specific)
- **path_index**: Fast path-to-node-id lookups
- **type_index**: Query optimization for node type filters
- **property_index**: Indexed property values for efficient filtering
- **reference_index**: Bidirectional cross-references

### RocksDB Column Families

The RocksDB implementation uses 16 specialized column families for optimal performance:

| Column Family | Purpose | Key Structure |
|---------------|---------|---------------|
| `NODES` | Node blob storage | `{tenant}\0{repo}\0{branch}\0{ws}\0nodes\0{node_id}\0{~rev}` |
| `PATH_INDEX` | Hierarchical path lookup | `{tenant}\0{repo}\0{branch}\0{ws}\0path\0{path}\0{~rev}` |
| `ORDERED_CHILDREN` | **Child ordering** | `{prefix}\0{parent_id}\0{order_label}\0{~rev}\0{child_id}` |
| `PROPERTY_INDEX` | Property-based queries | `{prefix}\0prop\0{name}\0{hash}\0{~rev}\0{node_id}` |
| `REFERENCE_INDEX` | Cross-references | Forward/reverse indexes |
| `TREES` | Content-addressed storage | Merkle tree nodes |
| + 10 more | Metadata, versions, FTS, etc. | Various patterns |

**Key Encoding Features:**
- **Null-byte separation** (`\0`) enables efficient prefix filtering and bloom filters
- **Descending revision encoding** (`~rev`) ensures newest entries sort first
- **Custom prefix transforms** optimize specific query patterns

---

## Natural Ordering with Fractional Indexing

### Overview

RaisinDB uses **fractional indexing** to maintain natural child ordering without requiring renumbering when items are inserted, reordered, or deleted. This is a key differentiator from traditional approaches that store order as integer positions.

### How It Works

**Order labels** are variable-length strings that maintain insertion order when sorted lexicographically:

```
Examples: "a0" → "a1" → "a2" → "a2m" → "a2m5"
```

**Key Operations (all O(1)):**
- **Append**: Generate next label (`"a0"` → `"a1"`)
- **Prepend**: Generate label before first (`"a0"` → `"Zz"`)
- **Insert between**: Calculate midpoint label (`"a0"` and `"a2"` → `"a1"`)
- **Reorder**: Assign new label without affecting siblings

### Storage Architecture

Child ordering is stored in a **separate column family** (`ORDERED_CHILDREN`), not embedded in node documents:

**Key Format:**
```
{tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0{order_label}\0{~rev}\0{child_id}
```

**Value Format:**
- Standard entries: Child name (UTF-8 string)
- Tombstones: Single byte `"T"` (marks deleted/moved children)
- Metadata: `\xFF\xFFMETA\0LAST` stores last label for append optimization

### Performance Benefits

**Tested with 50,000 children at root level:**

| Operation | Performance | Notes |
|-----------|-------------|-------|
| List children | 100,000+ nodes/sec | Prefix scan, naturally sorted |
| Reorder single child | 10-20ms | O(1), no sibling updates |
| Append child | ~1ms | Metadata cache enables O(1) |
| Insert between | ~1ms | Calculate midpoint label |

**Metadata Cache:** Stores last child's order label for ~10-20 microsecond append operations instead of milliseconds.

**Scalability:** The fragmented approach means parents with 100,000+ children can be efficiently queried without loading full child lists into memory.

---

## Node Content Model

### Immutable Node Documents

RaisinDB stores each node as an immutable document. Unlike traditional databases that overwrite data, RaisinDB creates a **new snapshot** for each revision that modifies a node. This enables:

- **Time-travel queries**: Access any historical version
- **Branch isolation**: Independent changes per branch
- **Safe deletions**: Deleted nodes remain accessible in historical revisions

A basic node document looks like this:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "my-article",
  "path": "/content/blog/my-article",
  "node_type": "Article",
  "properties": {
    "title": "Understanding RaisinDB",
    "author": "John Doe",
    "published": true
  },
  "created_at": "2025-10-21T10:30:00Z",
  "updated_at": "2025-10-21T14:22:00Z"
}
```

**Note:** The `children` array is **not stored** in the node document. Child ordering is maintained in the `ORDERED_CHILDREN` index for efficiency.

### Node Snapshots

Every commit creates immutable snapshots of changed nodes:

```rust
// Snapshot storage
/{tenant}/repo/{repo}/node/{node_id}/rev/{revision}

// Example: Node at revision 42
{
  "node_id": "550e8400-e29b-41d4-a716-446655440000",
  "revision": 42,
  "data": <serialized node JSON>,
  "timestamp": 1697456789
}
```

**Key benefits**:
- Snapshots are **immutable** (never modified after creation)
- Multiple revisions can reference the same snapshot (structural sharing)
- Empty snapshots mark deletions without removing data

---

## Revisions

Revisions in RaisinDB are **monotonically increasing integers** that represent a point-in-time snapshot of the repository. Each revision contains:

```rust
pub struct TreeCommitMeta {
    pub revision: u64,           // Sequential revision number
    pub parent_rev: Option<u64>, // Previous revision (ancestry)
    pub branch: String,           // Branch this commit belongs to
    pub workspace: String,        // Workspace identifier
    pub root_tree_id: String,    // Content-addressed root tree
    pub timestamp: i64,           // Unix timestamp
    pub message: String,          // Commit message
    pub actor: String,            // User who made the commit
    pub merge_info: Option<MergeInfo>, // Merge metadata if applicable
}
```

### Revision Ancestry

Revisions form a directed acyclic graph (DAG) through parent references:

```
main branch:
  rev 1 ← rev 2 ← rev 4 ← rev 6

develop branch (from rev 2):
  rev 1 ← rev 2 ← rev 3 ← rev 5
```

This enables:
- **Time-travel queries**: Navigate to any historical revision
- **Branch lineage**: Each branch maintains independent history
- **Merge base detection**: Find common ancestors for merging

---

## Content-Addressed Trees

### Tree Structure

RaisinDB uses **Merkle trees** to represent directory hierarchies. Each tree is content-addressed (ID derived from content hash), enabling:

- **Structural sharing**: Unchanged directories reused across revisions
- **Fast comparisons**: Different tree IDs = different content
- **Efficient storage**: Only store what changes

```rust
pub struct Tree {
    pub entries: Vec<TreeEntry>, // Ordered list of children (from fractional indexes)
}

pub struct TreeEntry {
    pub name: String,              // Child name
    pub node_id: String,           // Reference to node document
    pub node_type: String,         // Node type for filtering
    pub children_tree_id: Option<String>, // Subtree reference
}
```

**Important:** During commit, `Tree.entries` are built from the `ORDERED_CHILDREN` index, preserving the natural order established by fractional indexing.

### Structural Sharing Example

When one node changes in `/blog/2025/article-1`:

```
Revision 41 (before change):
  Root Tree (abc123)
    ├─ docs → Tree (def456)      ← unchanged
    ├─ blog → Tree (ghi789)       ← will change
    └─ images → Tree (mno345)     ← unchanged

Revision 42 (after change):
  Root Tree (xyz999)              ← new root
    ├─ docs → Tree (def456)      ← REUSED ✅
    ├─ blog → Tree (pqr111)       ← new tree
    │   └─ 2025 → Tree (stu222)   ← new tree
    │       └─ article-1 → Node (node3) ← changed
    └─ images → Tree (mno345)     ← REUSED ✅
```

**Result**: Only 3 new trees created, 2 trees reused. At 20,000 nodes, this is **10-20x faster** than rebuilding everything.

---

## Branches

### Branch Metadata

Branches in RaisinDB are **lightweight pointers** to revisions:

```rust
pub struct Branch {
    pub name: String,
    pub head: u64,                // Current HEAD revision
    pub created_at: i64,
    pub created_from: Option<u64>, // Parent revision
}
```

**Key properties**:
- Creating a branch is **instant** (just metadata, no copying)
- Branches share committed content via tree references
- Each branch maintains independent workspace deltas for draft changes

### Branch Isolation

RaisinDB achieves true Git-like branch isolation through:

1. **Workspace Deltas**: Draft changes stored per-branch
2. **Tree Snapshots**: Committed content shared across branches
3. **Revision Lineage**: Each branch tracks its own history

**Workflow**:
```
1. Create branch "develop" from main's HEAD (revision 10)
   → develop.head = 10
   → No data copied! ✅

2. Make changes on develop
   → Stored in workspace delta (branch-specific)
   → Not visible to other branches ✅

3. Commit develop (creates revision 11)
   → Promote delta to committed storage
   → Build trees from changed nodes and ORDERED_CHILDREN index
   → Update develop.head = 11
   → Clear workspace delta

4. main still at revision 10
   → Independent of develop's changes ✅
```

---

## Performance Characteristics

### RocksDB Throughput (SDK Client Connection)

**Tested Configuration:**
- 50,000 nodes at root level (flat structure)
- RocksDB with 16 column families
- Atomic WriteBatch operations
- Natural ordering with fractional indexing

| Operation | Throughput | Latency | Notes |
|-----------|-----------|---------|-------|
| **Node Creation** | **~4,000 nodes/sec** | 0.25ms/node | Real-world with SDK client |
| **Optimized Creation** | 2,000+ nodes/sec | 0.5ms/node | Using `add()` fast path |
| **Node Listing** | 100,000+ nodes/sec | 0.01ms/node | Prefix scan with MVCC |
| **Child Reordering** | N/A | 10-20ms | Single operation |
| **Delete Operation** | N/A | 5-10ms | Tombstone write |

**Real-World Performance:**
- **Network overhead** (10-50ms per operation) typically dominates RocksDB latency
- **Batch operations** recommended for bulk inserts (use transactions)
- **Metadata caching** provides ~10x speedup for sequential appends

### Detailed Timing Breakdown

**Single Node Creation (847 microseconds total):**
```
├── Revision allocation: 42μs
├── Serialization: 15μs
├── Index preparation: 120μs
├── Order label calculation: 380μs
│   ├── Parent lookup: 25μs
│   ├── Metadata cache hit: 15μs
│   └── Batch preparation: 310μs
├── RocksDB write: 250μs
└── Revision indexing: 40μs
```

### Structural Sharing (Implemented)

**Problem**: At 20,000 nodes, naive tree snapshotting writes 100+ trees per commit, even for 1-node changes.

**Solution**: Incremental tree building with structural sharing.

**Performance gain**:

| Scenario | Naive | Optimized | Improvement |
|----------|-------|-----------|-------------|
| Change 1 node (20k total) | 2-5s | &lt;100ms | **20-50x** |
| Change 100 nodes | 3-6s | &lt;500ms | **6-12x** |
| Reorder children | 2-5s | &lt;100ms | **20-50x** |

---

## Node Deletion Semantics

RaisinDB **never physically deletes nodes**. Instead, deletions are represented as **empty snapshots** and **tombstones**:

```rust
// Node exists at revision 41:
/{tenant}/repo/{repo}/node/{node_id}/rev/41 → <node data>

// Node deleted at revision 42:
/{tenant}/repo/{repo}/node/{node_id}/rev/42 → <empty>

// ORDERED_CHILDREN entry marked with tombstone:
{parent_id}\0{order_label}\0{~rev}\0{child_id} → "T"

// Node still accessible at revision 41! ✅
```

**Benefits**:
- Historical revisions remain accessible
- Other branches unaffected by deletions
- Time-travel queries work correctly
- Enables "undelete" operations

---

## Configuration Best Practices

### RocksDB Tuning

```rust
use rocksdb::{Options, BlockBasedOptions, Cache};

pub fn create_rocks_options() -> Options {
    let mut opts = Options::default();
    opts.create_if_missing(true);

    // Increase block cache (hot data in memory)
    let cache = Cache::new_lru_cache(256 * 1024 * 1024); // 256 MB
    let mut block_opts = BlockBasedOptions::default();
    block_opts.set_block_cache(&cache);
    block_opts.set_bloom_filter(10.0, false);

    opts.set_block_based_table_factory(&block_opts);
    opts.set_write_buffer_size(64 * 1024 * 1024); // 64 MB

    opts
}
```

### Monitoring and Metrics

**Key Metrics to Track:**
- **Commit latency**: Target &lt;100ms for typical commits
- **Tree reuse rate**: Should be >80% with structural sharing
- **Snapshot count per node**: Monitor for unbounded growth
- **Child list size**: Track largest parents (consider partitioning at 100k+ children)
- **Order label length**: Warn if labels approach 48 characters

---

## Future Enhancements

### Planned Features

1. **PostgreSQL/MySQL backends**: Relational database storage option
2. **Conflict detection**: Three-way merge with conflict resolution
3. **Revision GC**: Automatic cleanup of old revisions
4. **Advanced caching**: LRU cache for hot trees and nodes
5. **Distributed deployment**: Multi-node clustering with lease management

### Research Topics

- **CRDT integration**: Conflict-free replicated data types for offline editing
- **Lazy tree loading**: On-demand tree traversal for large repositories
- **Compression**: Delta compression between similar snapshots

---

## Conclusion

RaisinDB's document storage architecture provides a robust, Git-like versioning system optimized for hierarchical content management. Key strengths:

✅ **True branch isolation** via workspace deltas
✅ **Efficient storage** through structural sharing
✅ **Natural ordering** with O(1) child reordering (fractional indexing)
✅ **Fragmented indexes** enable scalability to 100k+ children
✅ **Time-travel queries** via immutable snapshots
✅ **Fast commits** (10-20x improvement with incremental trees)
✅ **Safe deletions** (nodes never physically removed)
✅ **Production throughput** (~4,000 nodes/sec with SDK client)

The system is production-ready for single-node deployments with proven performance at scale (tested with 50,000 children).

---

## See Also

- [Database Comparisons](/docs/why/architecture/comparisons)
- [Architecture Overview](/docs/why/architecture)
- [API Reference](/docs/access/rest/overview)
- [Query Capabilities](/docs/access/sql/overview)

---

*© 2025 RaisinDB Contributors*
