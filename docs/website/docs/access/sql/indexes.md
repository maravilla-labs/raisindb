---
title: Indexes and fast reads
description: Which predicates RaisinDB answers from an index, what a package has to declare (usually nothing), and how to check a plan.
---

# Indexes and fast reads

Most of what a website reads — a page by path, a URL by its localized slug, a
redirect by its source, a folder's children in editorial order — is answered by
an index that already exists. This page lists them, says what a package has to
declare (usually nothing), and shows how to check.

## Check the plan first

Prefix any `SELECT` with `EXPLAIN`. The leaf operator tells you how the rows are
found:

```sql
EXPLAIN SELECT id, path FROM stories WHERE properties->>'url_fr'::String = $1
```

```
Project: 2 expressions
  Filter: 1 predicates
    PropertyIndexScan: url_fr=/fr/vols
```

| leaf operator | found by | cost |
|---|---|---|
| `PathIndexScan` | the path index | one lookup per path |
| `PropertyIndexScan` | the property index | one seek per matching value |
| `CompoundIndexScan` | a declared compound index (workspace or NodeType) | one seek, rows already ordered |
| `PrefixScan` | the editorial-order index (`CHILD_OF`, `DESCENDANT_OF`) | proportional to the subtree |
| `ReferenceIndexScan` | the reverse reference index (`REFERENCES(...)`) | proportional to the backlinks |
| `TableScan` | every node in the workspace | proportional to the workspace — avoid on hot paths |

## What is indexed without declaring anything

**Every top-level property of every node** is written to the property index on
every write, together with `node_type`, `name` and `archetype`. There is no
`indexed: true` to set. An equality on any of them is a seek:

```sql
-- a localized URL resolved in one query (url_fr stored on the base node)
SELECT id, path FROM stories WHERE properties->>'url_fr'::String = $1

-- a redirect by its source path
SELECT properties FROM redirects
WHERE node_type = 'site:Redirect' AND properties->>'source'::String = $1
```

Write the predicate with the `::String` cast. Both spellings plan the same way,
and the cast one is always correct when combined with other predicates.
`->>` yields text, so compare a number-valued property against a string
(`properties->>'seq'::String = '0'`).

**Paths** have their own index. `path = $1` is one lookup, and so is each entry
of `path IN (...)`:

```sql
EXPLAIN SELECT id, path FROM assets WHERE path IN ('/a.jpg', '/b.jpg')
```

```
Project: 2 expressions
  Union: 2 branch(es)
    PathIndexScan: path=/a.jpg
    PathIndexScan: path=/b.jpg
```

Use `IN` to read a known set of nodes in one round trip rather than one query per
node.

**Children and subtrees** are read from the editorial-order index, already in
the order editors arranged them. `CHILD_OF` reads one parent's children, and
`DESCENDANT_OF` walks a subtree depth-first. Add no `ORDER BY` when you want
editorial order: the rows already come out that way, and `ORDER BY path` would
re-sort them alphabetically. See [Editorial ordering](./editorial-ordering.md).

## What is NOT indexed

- **Nested properties.** `properties->>'seo.slug'` and array elements are
  evaluated per row. If a value is looked up by equality on a hot path, store it
  as a top-level property.
- **Translated values.** The index holds the base-language value. A predicate on
  a translatable field combined with `locale = 'fr'` finds nodes by their BASE
  value and then checks the translated one, so a node whose French value
  matches but whose base value does not is never found. Store lookup keys that
  differ per language as separate, untranslated base properties (`url_de`,
  `url_fr`, `url_en`), and filter on those.
- **Ranges and `LIKE` on properties.** Only equality seeks the property index. A
  `LIKE 'prefix%'` on `path` is a prefix scan and is fine.

## Declaring a compound index

A compound index is only needed when a query filters on one value AND orders
by another — for example the newest items in a folder. It is declared on the
**workspace** (it then holds every node of the workspace, whatever its type) or
on a **NodeType** (it then holds only that type's nodes). The most common one,
a folder listing by creation time, is **built in**: you do not declare it.

### Built in: newest- and oldest-first folder listings

Every workspace carries a built-in index on `(__parent_path, __created_at)`,
so a folder listing by creation time is index-served out of the box — newest
or oldest first, with or without a `LIMIT`, typed or untyped, with or without
other predicates:

```sql
SELECT id, name FROM stories
WHERE CHILD_OF('/site/news')
ORDER BY created_at DESC LIMIT 20
```

```
Limit: limit=20, offset=0
  Project: 2 expressions
    CompoundIndexScan: @__children_by_created_at [__parent_path=/site/news] index-order limit_hint=20 (owner: workspace stories)
```

It costs one index entry per node: written on create, re-keyed on move, ended
on delete. An update that changes neither the parent nor `created_at` writes
nothing to it. There is no built-in `updated_at` index (it would be rewritten
on every update), and `CHILD_OF` with `ORDER BY __order` or with no `ORDER BY`
keeps the editorial order. `DESCENDANT_OF` does not use it: a subtree is a path
range, not one parent.

The index is built in the background on every node, per branch, after an
upgrade or when a workspace is created: until it is ready on that node, the
listing scans and returns the same rows more slowly. An index you declare
yourself that matches a query as well or better is preferred over it.

**Opting out.** A workspace that never lists folders by creation time (an
append-only log, say) can switch it off in its configuration:

```yaml
name: audit_log
config:
  builtin_indexes:
    children_by_created_at: false
```

or over SQL (`NULL` restores the default, on):

```sql
UPDATE Workspaces
SET builtin_indexes = '{"children_by_created_at": false}'::jsonb
WHERE name = 'audit_log'
```

The same `config.builtin_indexes` object is accepted by the workspace API.
Switching it off takes effect at once (the planner stops using it), and the
background job then deletes its entries. Switching it back on builds it again
in the background. `SELECT builtin_indexes FROM Workspaces` shows the switches
in force. Index names starting with `__` are reserved for built-in indexes and
cannot be declared.

### On the workspace: folder listings

A folder listing usually names no node type. Declare the index on the
workspace, in its YAML (`workspaces/stories.yaml` in a package) or through the
workspace API:

```yaml
name: stories
compound_indexes:
  - name: folder_recent
    columns:
      - property: __parent_path
        column_type: String
      - property: __created_at
        column_type: Timestamp
    has_order_column: true
```

```sql
SELECT id, name FROM stories
WHERE CHILD_OF('/site/news')
ORDER BY created_at DESC LIMIT 20      -- every child, whatever its type
```

```
Limit: limit=20, offset=0
  Project: 2 expressions
    CompoundIndexScan: @folder_recent [__parent_path=/site/news] index-order limit_hint=20 (owner: workspace stories)
```

The `@` marks a workspace index: it lives in its own keyspace, so a NodeType
index of the same name never shares entries with it. The same declaration can be
made over SQL:

```sql
UPDATE Workspaces
SET compound_indexes = '[{"name":"folder_recent","columns":[
      {"property":"__parent_path","column_type":"String"},
      {"property":"__created_at","column_type":"Timestamp"}],
    "has_order_column":true}]'::jsonb
WHERE name = 'stories'
```

A workspace index matched on `__parent_path` alone is used only when it also
serves the `ORDER BY`. `CHILD_OF('/x') ORDER BY __order` (editorial order) and a
`CHILD_OF` with no `ORDER BY` keep reading the editorial-order index, in the
order editors arranged.

### On a NodeType: listings of one type

```yaml
name: site:NewsItem
compound_indexes:
  - name: site_news_recent
    columns:
      - property: __parent_path
        column_type: String
      - property: __created_at
        column_type: Timestamp
    has_order_column: true
```

```sql
SELECT name FROM stories
WHERE CHILD_OF('/site/news') AND node_type = 'site:NewsItem'
ORDER BY created_at DESC LIMIT 10
```

A NodeType index serves only a query scoped to that type (`node_type = ...` or
`IS_A(...)`). An untyped query never uses it: it would silently miss every node
of another type. Names starting with `@` are reserved for workspace indexes.

### For both

Every column is an object with an explicit `column_type`. `String` equality
columns (including `__parent_path` and `__node_type`) and a trailing
`Timestamp` order column (`__created_at`, `__updated_at`) are supported;
`__order` is not an index column. A declared index is not used until it is
built: declaring or changing one queues a build on every node, and until it
finishes the query scans and returns the same rows more slowly. `EXPLAIN` shows
`CompoundIndexScan` once it is ready.

## Reading referenced nodes

`RESOLVE(properties, depth)` inlines referenced nodes, reading each distinct
node once. What it costs is mostly what it inlines, so name the fields the page
renders:

```sql
SELECT id, path, RESOLVE(properties, 2, 'title,alt,file,renditions') AS properties
FROM stories WHERE path = $1
```

See [RESOLVE](./raisinsql.md#resolve).
