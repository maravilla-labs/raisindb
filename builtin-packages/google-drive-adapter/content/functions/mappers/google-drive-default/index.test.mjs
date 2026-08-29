// Run with: node --test .../mappers/google-drive-default/index.test.mjs
//
// The mapper is loaded as the engine loads it: a bare script whose entry point
// is the global `handler`. It performs no I/O, so no host is injected.
//
// The defect these cover: `to_external` had no folder branch. A locally-created
// raisin:Folder translated to `{ name }` alone, and Drive makes a FILE from a
// body with no mime type — so a mirrored folder arrived as a zero-byte document
// wearing the folder's name, and every node the engine then tried to create
// inside it had nowhere to go.

import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8')
const handler = new Function(`${src}\nreturn handler;`)()

const FOLDER_MIME = 'application/vnd.google-apps.folder'

test('a folder CREATE carries the mime type that makes Drive create a folder', () => {
  const out = handler({
    operation: 'to_external',
    intent: 'create',
    node: { node_type: 'raisin:Folder', name: 'Gründung', properties: { title: 'Gründung' } },
    fields: null,
  })
  assert.deepEqual(out.payload, { name: 'Gründung', mimeType: FOLDER_MIME })
})

test("a folder with no title falls back to the node's own name", () => {
  // A raisin:Folder routinely carries no `title`, and the adapter refuses a
  // nameless create.
  const out = handler({
    operation: 'to_external',
    intent: 'create',
    node: { node_type: 'raisin:Folder', name: 'Archiv', properties: {} },
  })
  assert.equal(out.payload.name, 'Archiv')
})

test('a folder UPDATE is a plain rename and must not carry a mime type', () => {
  // A mimeType on a PATCH is at best a no-op revision — and a revision per file
  // per drain is what the empty-PATCH guard exists to prevent.
  const out = handler({
    operation: 'to_external',
    intent: 'update',
    node: { node_type: 'raisin:Folder', name: 'Archiv', properties: { title: 'Archiv 2026' } },
    fields: ['title'],
  })
  assert.deepEqual(out.payload, { name: 'Archiv 2026' })
})

test('a file update emits only the allow-listed field', () => {
  const out = handler({
    operation: 'to_external',
    intent: 'update',
    node: {
      node_type: 'raisin:Asset',
      name: 'a.txt',
      properties: { title: 'a.txt', size: 12, web_url: 'https://x', __external_id: 'F1' },
    },
    fields: ['title'],
  })
  assert.deepEqual(out.payload, { name: 'a.txt' })
  assert.equal(out.external_id, 'F1')
})

test('a blank title is not an instruction to clear the name', () => {
  // Drive has no nameless file, and sending "" renames it to nothing.
  const out = handler({
    operation: 'to_external',
    intent: 'update',
    node: { node_type: 'raisin:Asset', name: 'a.txt', properties: { title: '' } },
    fields: ['title'],
  })
  assert.equal(out, null)
})

test('to_node still maps a folder and a file, and an absent operation means to_node', () => {
  const folder = handler({
    external_item: { external_id: 'D1', name: 'docs', is_folder: true },
  })
  assert.equal(folder.node_type, 'raisin:Folder')

  const file = handler({
    operation: 'to_node',
    external_item: {
      external_id: 'F1',
      name: 'sheet',
      is_folder: false,
      mime_type: 'application/vnd.google-apps.spreadsheet',
    },
  })
  assert.equal(file.node_type, 'raisin:Asset')
  assert.equal(file.properties.provider_kind, 'google-sheet')
})
