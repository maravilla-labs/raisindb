---
title: Editorial ordering
description: Query, sort and paginate by manual drag-and-drop order using the __order and __tree_order columns.
---

# Editorial ordering

RaisinDB tracks a **manual order** for every parent's children — the order your
editors set by dragging in the admin console. It is stored as a fractional index,
so inserting between two siblings never renumbers anything.

Two columns expose it to SQL:

| column | orders | use it for |
|---|---|---|
| `__order` | a node among **its siblings** | listing/paging one parent's children |
| `__tree_order` | a node within **a whole subtree** | listing/paging an entire tree in document order |

Both are opaque, lexicographically sortable text. Read them, sort by them, and
pass them back as pagination cursors — but never parse or construct one.

## Listing children in editorial order

A hierarchy read already returns editorial order, so the `ORDER BY` is optional —
but stating it is better, because it pins the order instead of relying on which
scan the planner picked:

```sql
SELECT name, __order
  FROM 'content'
 WHERE CHILD_OF('/menu')
 ORDER BY __order
```

`ORDER BY __order DESC` reverses it.

## Keyset pagination

Pass the last row's `__order` back as an exclusive cursor. This seeks straight
into the index — cost is proportional to the page, not to the number of children:

```sql
-- page 1
SELECT name, __order FROM 'content'
 WHERE CHILD_OF('/menu')
 ORDER BY __order LIMIT 20;

-- page 2 ($1 = the last row's __order from page 1)
SELECT name, __order FROM 'content'
 WHERE CHILD_OF('/menu') AND __order > $1
 ORDER BY __order LIMIT 20;
```

Use a **bound parameter**, not string interpolation — the value is opaque and
must round-trip verbatim.

## Paging a whole subtree

`__tree_order` sorts an entire subtree into document order: each node appears
before its descendants, and a subtree stays contiguous and in editorial order.

```sql
SELECT path, __tree_order FROM 'content'
 WHERE DESCENDANT_OF('/menu') AND __tree_order > $1
 ORDER BY __tree_order LIMIT 20;
```

Resuming is cheap: the cursor is decoded back into per-level positions and the
walk descends straight to where it left off, rather than re-walking the subtree.

Two limits worth knowing:

- **Ascending only.** `ORDER BY __tree_order DESC` works but is sorted in memory;
  reversing document order means reversing every sibling group at every level, so
  the traversal can't do it directly.
- **Only tree scans populate it.** On a scan that isn't a traversal (a property
  index lookup, say) `__tree_order` is `NULL` rather than guessed.

## `__order` is not `path`

This is the easy mistake. Both order parents before children, so they look
interchangeable — but they order *siblings* differently:

- **`path`** sorts siblings **alphabetically by name**, like a filesystem listing.
- **`__order` / `__tree_order`** sort siblings by **editorial order**.

With children placed in the order `c`, `a`, `b`:

```sql
ORDER BY path         -- a, b, c   (alphabetical; the manual order is discarded)
ORDER BY __tree_order -- c, a, b   (the order the editor set)
```

They agree only when the editorial order happens to be alphabetical — which is
exactly why using `path` by mistake can look fine until someone drags something.

Never mix them: `WHERE __tree_order > $1 ORDER BY path` advances the cursor in one
order while sorting in another, which drops and duplicates rows. Keyset pagination
requires the cursor column and the `ORDER BY` column to be the same.

## Changing the order

Ordering is a write operation, not a property edit — the server assigns the key,
you name a position or a neighbour.

```ts
// JS SDK — children are named, not full paths
await ws.nodes().reorder('/menu', 'about', 0);              // move to the front
await ws.nodes().moveChildBefore('/menu', 'about', 'home');
await ws.nodes().moveChildAfter('/menu', 'about', 'contact');
```

```js
// Functions runtime
raisin.nodes.reorderChild('content', '/menu', 'about', 0);
raisin.nodes.moveChildBefore('content', '/menu', 'about', 'home');
raisin.nodes.moveChildAfter('content', '/menu', 'about', 'contact');
```

```http
POST /api/repository/{repo}/{branch}/head/{ws}/menu/about/raisin:cmd/reorder
{ "targetPath": "/menu/home", "movePosition": "before" }
```

All of these return the node with its newly assigned `order_key`, which is the
same value `__order` reports.

A reorder records a revision, so it appears in the node's history and replicates
like any other write.

## Don't hand-roll a `sort_order` property

A numeric `sort_order` property is a common workaround, and it costs you:
inserting between `1` and `2` forces a renumber, concurrent edits collide, and you
have to keep it consistent yourself. The built-in ordering has none of those
problems, is what the admin console's drag-and-drop already writes to, and is
keyset-pageable. Prefer it.
