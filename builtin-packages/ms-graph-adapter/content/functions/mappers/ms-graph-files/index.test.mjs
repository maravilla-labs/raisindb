// SPDX-License-Identifier: BSL-1.1
//
// Files-mapper tests. Run with `node --test index.test.mjs`.
//
// This mapper had no tests at all, and it is the one that decides whether a
// drive item becomes a folder or an asset and what a consuming app can read off
// the node.

import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

// index.js is loaded the way the ENGINE loads it — a bare script whose entry
// point is the global `handler` — so there is nothing to import. Testing it any
// other way would test a module the runtime never sees.
const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8')
const handler = new Function(`${src}\nreturn handler;`)()

const MOUNT = { mount_id: 'm1', mount_path: '/drive', sync_config: { resource: 'files' } }

function item(extra) {
  return {
    external_id: '01ABCDEF',
    name: '01ABCDEF', // external_id and name are the Graph id, never the filename
    is_folder: false,
    metadata: { filename: 'Quarterly Report.pdf' },
    ...extra,
  }
}

const call = (external_item) => handler({ external_item, mount: MOUNT })
const push = (node, extra) =>
  handler({ operation: 'to_external', node, mount: MOUNT, ...extra })

test('a file becomes a raisin:Asset titled with its real filename', () => {
  const node = call(
    item({
      mime_type: 'application/pdf',
      size_bytes: 12345,
      web_url: 'https://contoso.sharepoint.com/x.pdf',
      parent_id: 'PARENT-1',
      created_at: '2026-01-01T00:00:00Z',
      modified_at: '2026-02-02T00:00:00Z',
    })
  )
  assert.equal(node.node_type, 'raisin:Asset')
  // The id keys the node; the HUMAN name is what a person reads.
  assert.equal(node.name, 'Quarterly Report.pdf')
  assert.equal(node.properties.title, 'Quarterly Report.pdf')
  assert.equal(node.properties.mimeType, 'application/pdf')
  assert.equal(node.properties.size, 12345)
  assert.equal(node.properties.parent_id, 'PARENT-1')
  assert.equal(node.properties.provider, 'ms-graph')
  assert.equal(node.properties.provider_kind, 'file')
})

test('a folder becomes a raisin:Folder', () => {
  const node = call(item({ is_folder: true, metadata: { filename: 'Reports' } }))
  assert.equal(node.node_type, 'raisin:Folder')
  assert.equal(node.name, 'Reports')
  assert.equal(node.properties.icon, 'folder')
})

test('an item with no external_id is skipped rather than half-mapped', () => {
  assert.equal(call({ name: 'x', is_folder: false }), null)
  assert.equal(handler({ mount: MOUNT }), null)
})

test('a missing filename falls back to the id, so a node is never nameless', () => {
  const node = call(item({ metadata: {} }))
  assert.equal(node.name, '01ABCDEF')
})

test('the mapper is PURE: identical input maps to identical output', () => {
  // A mapper that answers differently for the same item rewrites the node on
  // every remap and defeats any content-based dedup. This is why the
  // short-lived download_url carries no minted capture timestamp — staleness
  // is read from the engine's own __synced_at instead.
  const a = call(item({ download_url: 'https://dl.example/one-hour-link' }))
  const b = call(item({ download_url: 'https://dl.example/one-hour-link' }))
  assert.deepEqual(a, b)
})

test('web_url and download_url are carried, and neither is the content path', () => {
  const node = call(
    item({
      web_url: 'https://contoso.sharepoint.com/x.pdf',
      download_url: 'https://dl.example/pre-authenticated',
    })
  )
  // Durable: the item's page.
  assert.equal(node.properties.web_url, 'https://contoso.sharepoint.com/x.pdf')
  // Short-lived (~1h) and kept only as a convenience — the bytes come from
  // `get_content`, which mints a fresh URL per call.
  assert.equal(node.properties.download_url, 'https://dl.example/pre-authenticated')
  // No bytes are ever inlined during sync.
  assert.equal(node.properties.content, undefined)
  assert.equal(node.properties.file, undefined)
})

test('provider metadata is passed through verbatim', () => {
  const meta = { filename: 'a.txt', ctag: 'c1', parent_path: '/drive/root:/docs' }
  const node = call(item({ metadata: meta }))
  assert.deepEqual(node.properties.provider_metadata, meta)
})

// ---- to_external ----------------------------------------------------------
//
// The write half. It emits driveItem METADATA only — the bytes travel beside
// the payload as the engine's `content` — and the one thing it must never do is
// answer null for a push that has bytes to send.

test('a create names the file and never overwrites a stranger\'s', () => {
  const out = push(
    { node_type: 'raisin:Asset', name: 'report.pdf', properties: { title: 'report.pdf' } },
    { intent: 'create' }
  )
  assert.equal(out.payload.name, 'report.pdf')
  // `rename`, not `replace`: a locally-born node lands in a drive full of
  // documents this mount never imported, and replace would destroy one and
  // report success. Graph answers with the real item and the engine adopts it.
  assert.equal(out.payload['@microsoft.graph.conflictBehavior'], 'rename')
})

test('an update replaces the item it addresses by id', () => {
  const out = push(
    { node_type: 'raisin:Asset', name: 'report.pdf', properties: { title: 'report.pdf' } },
    { intent: 'update' }
  )
  assert.equal(out.payload['@microsoft.graph.conflictBehavior'], 'replace')
})

test('a content-only push is never dropped', () => {
  // The engine SKIPS an item whose to_external answers null, so returning null
  // for an empty payload — the way the calendar mapper does — would silently
  // drop every push whose only change is the file's bytes.
  const out = push(
    { node_type: 'raisin:Asset', name: 'report.pdf', properties: {} },
    { intent: 'update', fields: ['nothing_writable'] }
  )
  assert.notEqual(out, null)
  assert.ok(out.payload['@microsoft.graph.conflictBehavior'])
})

test('a folder create announces itself; a folder rename is an ordinary patch', () => {
  // The adapter falls back to "no bytes means folder", but the mapper knows the
  // node type and says so rather than making the adapter infer it.
  const folder = { node_type: 'raisin:Folder', name: 'Reports', properties: { title: 'Reports' } }
  const created = push(folder, { intent: 'create' })
  assert.equal(created.payload.is_folder, true)
  assert.equal(created.payload.name, 'Reports')

  const renamed = push(folder, { intent: 'update' })
  assert.equal(renamed.payload.name, 'Reports')
  assert.equal(renamed.payload.is_folder, undefined, 'a rename is not a create')
})

test('a create with no name at all is declined rather than guessed', () => {
  assert.equal(push({ node_type: 'raisin:Asset', properties: {} }, { intent: 'create' }), null)
  assert.equal(push(null, { intent: 'create' }), null)
})

test('mapper_capabilities reports the write half, and to_node still works', () => {
  assert.deepEqual(handler({ operation: 'mapper_capabilities', mount: MOUNT }), {
    to_external: true,
  })
  // An absent operation is to_node, so the read path is unchanged.
  assert.equal(call(item({ is_folder: true })).node_type, 'raisin:Folder')
  assert.equal(handler({ operation: 'to_node', external_item: item(), mount: MOUNT }).node_type,
    'raisin:Asset')
})

test('a mount may name its own folder types, and the mapper honours them', () => {
  // The product's container type never appears in the engine or this mapper —
  // the MOUNT names it, and both sides read the same list.
  const studioFolder = {
    node_type: 'studio:Folder',
    name: 'Reports',
    properties: { title: 'Reports' },
  }
  const withoutConfig = handler({
    operation: 'to_external',
    node: studioFolder,
    mount: MOUNT,
    intent: 'create',
  })
  assert.equal(withoutConfig.payload.is_folder, undefined, 'unconfigured: just a node')

  const configured = handler({
    operation: 'to_external',
    node: studioFolder,
    mount: { ...MOUNT, sync_config: { ...(MOUNT.sync_config || {}), folder_node_types: ['studio:Folder'] } },
    intent: 'create',
  })
  assert.equal(configured.payload.is_folder, true)
  assert.equal(configured.payload.name, 'Reports')
})
