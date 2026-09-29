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
| `CompoundIndexScan` | a declared compound index | one seek, rows already ordered |
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
NodeType:

```yaml
name: site:NewsItem
compound_indexes:
  - name: site_news_recent        # branch-global: prefix it with the type
    columns:
      - property: __parent_path
        column_type: String
      - property: __created_at
        column_type: Timestamp
    has_order_column: true
```

```sql
SELECT name FROM stories
WHERE CHILD_OF('/site/news')
ORDER BY created_at DESC LIMIT 10     -- a CompoundIndexScan once declared
```

Every column is an object with an explicit `column_type`. Only `String`
equality columns and a trailing `Timestamp` order column are supported, and
nodes written before the index was declared need a rebuild before they appear.

## Reading referenced nodes

`RESOLVE(properties, depth)` inlines referenced nodes, reading each distinct
node once. What it costs is mostly what it inlines, so name the fields the page
renders:

```sql
SELECT id, path, RESOLVE(properties, 2, 'title,alt,file,renditions') AS properties
FROM stories WHERE path = $1
```

See [RESOLVE](./raisinsql.md#resolve).
