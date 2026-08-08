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
