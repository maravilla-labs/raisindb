# SDK: Node Operations

Nodes are the fundamental data unit in RaisinDB. All node operations are scoped to a workspace.

## Setup

```typescript
const db = client.database('my-repo');
const ws = db.workspace('content');
const nodes = ws.nodes();
```

## CRUD Operations

### Create

```typescript
const node = await nodes.create({
  type: 'Page',
  path: '/pages/home',
  properties: { title: 'Home Page', published: true },
});
```

### Get

```typescript
// By ID
const node = await nodes.get('node-uuid');

// By path
const node = await nodes.getByPath('/pages/home');
```

### Update

```typescript
const updated = await nodes.update('node-uuid', {
  properties: { title: 'Updated Title' },
});
```

### Delete

```typescript
await nodes.delete('node-uuid');
```

## Query

```typescript
// By type
const pages = await nodes.queryByType('Page', 100);

// By property
const published = await nodes.queryByProperty('published', true);

// Custom query
const results = await nodes.query({
  query: { node_type: 'Page', path: '/pages/*' },
  limit: 50,
  offset: 0,
});
```

## Tree Operations

```typescript
// List direct children
const children = await nodes.listChildren('/pages');

// Get full tree (nested)
const tree = await nodes.getTree('/pages', 3);  // max depth 3

// Get flat tree (array in tree order)
const flat = await nodes.getTreeFlat('/pages');

// Get children by path
const kids = await nodes.getChildrenByPath('/pages');
```

## Move, Copy, Rename, Reorder

```typescript
await nodes.move('/pages/old-page', '/archive');
await nodes.rename('/pages/draft', 'published-page');
await nodes.copy('/templates/base', '/pages', 'new-page');
await nodes.copyTree('/sections/header', '/pages/home');  // deep copy

// Reorder siblings
await nodes.reorder('/pages/about', 'Vz');  // base62 fractional index
await nodes.moveChildBefore('/pages', '/pages/about', '/pages/home');
await nodes.moveChildAfter('/pages', '/pages/about', '/pages/contact');
```

## Relationships

```typescript
// Add a relation
await nodes.addRelation('/pages/home', 'references', '/pages/about');
await nodes.addRelation('/pages/home', 'references', '/pages/faq', { weight: 0.8 });

// Remove a relation
await nodes.removeRelation('/pages/home', '/pages/about');

// Get all relationships
const rels = await nodes.getRelationships('/pages/home');
// rels.incoming: edges pointing to this node
// rels.outgoing: edges from this node
```

## Transactions

Group multiple operations into an atomic commit:

```typescript
const tx = ws.transaction();
await tx.begin({ message: 'Create initial content' });

try {
  await tx.nodes().create({ type: 'Page', path: '/home', properties: { title: 'Home' } });
  await tx.nodes().create({ type: 'Page', path: '/about', properties: { title: 'About' } });
  await tx.commit();
} catch (error) {
  await tx.rollback();
  throw error;
}
```
