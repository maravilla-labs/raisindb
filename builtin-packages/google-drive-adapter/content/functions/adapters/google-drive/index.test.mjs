// Run with: node --test .../adapters/google-drive/index.test.mjs
//
// index.js is loaded the way the ENGINE loads it — a bare script whose entry
// point is the global `handler`, with `raisin` injected by the host — so there
// is nothing to import and the file must stay a single module-free script.
// (`tests_google_drive_write.rs` hands QuickJS this one file with an empty
// module map; a sibling `import` would resolve to nothing at run time.)
//
// Every case here is a defect this adapter actually shipped:
//
//  * `get_changes` emitted a FLAT `item.name` as relative_path while the full
//    walk emits a nested one. The engine's remap then MOVED the node on every
//    disagreeing run — out of its folder on a delta, back on the next full
//    reconcile — forever. It is the same bug that was fixed for ms-graph, with
//    two Drive-only twists: the changes feed is account-wide, so the parent walk
//    is also the subtree filter, and a legacy file can have several parents.
//  * `create` read `params.name` / `params.is_folder` / `params.mime_type` /
//    `params.content`-as-a-string. The engine sends none of those: it sends
//    `payload` / `parent_id` / `parent_external_id` / `relative_path` and
//    `content` as an OBJECT. So the name was undefined, the folder branch never
//    ran, and the multipart body carried the string "[object Object]".
//  * the whole content path went through a multipart/related body the host
//    binding cannot assemble around raw bytes.

import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8')
const load = new Function('raisin', `${src}\nreturn handler;`)

const CREDENTIAL = { access_token: 'tok' }
const MOUNT = {
  mount_id: 'm1',
  mount_path: '/drive',
  remote_root: 'ROOT',
  sync_config: {},
}

/**
 * A handler whose fetch answers from a queue, recording every request.
 *
 * A response is `{ status?, headers?, body? }`; an exhausted queue throws, so a
 * test that expects N calls proves the adapter made no more than N.
 */
function stub(responses) {
  const calls = []
  const queue = [...responses]
  const handler = load({
    http: {
      fetch(url, request) {
        calls.push({ url, request })
        const next = queue.shift()
        if (!next) throw new Error(`unexpected extra request: ${url}`)
        return { status: 200, headers: {}, body: {}, ...next }
      },
    },
  })
  return { handler, calls }
}

const ok = (body) => ({ status: 200, headers: {}, body })

// ---- get_changes: the path must be the one the full walk produces ----------

/** A change page with one changed file, then the parent lookups it needs. */
function deltaFor(file, parents) {
  const responses = [ok({ changes: [{ fileId: file.id, file }], newStartPageToken: 't2' })]
  for (const p of parents) responses.push(ok(p))
  return responses
}

test('a delta item carries its FOLDER PATH, not a flat name', () => {
  const file = { id: 'F1', name: 'report.pdf', mimeType: 'application/pdf', parents: ['B'], version: '7' }
  const { handler, calls } = stub(
    deltaFor(file, [
      { id: 'B', name: 'b', parents: ['A'] },
      { id: 'A', name: 'a', parents: ['ROOT'] },
    ])
  )
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { since_token: 't1' },
  })

  assert.equal(out.items.length, 1)
  assert.equal(
    out.items[0].relative_path,
    'a/b/report.pdf',
    'the full walk materializes this at a/b/report.pdf; a flat name here makes ' +
      'the engine relocate the node on every run that disagrees'
  )
  assert.equal(out.next_token, 't2')
  assert.equal(calls.length, 3, 'the page plus one lookup per unseen ancestor folder')
})

test('a file directly in the mount root keeps a bare name', () => {
  const file = { id: 'F1', name: 'top.txt', parents: ['ROOT'] }
  const { handler } = stub(deltaFor(file, []))
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { since_token: 't1' },
  })
  assert.equal(out.items[0].relative_path, 'top.txt')
})

test("an account-wide change outside the mount is dropped, not materialized", () => {
  // Drive's changes feed reports EVERY file the account can see. Nothing else
  // keeps a stranger's file out of the mount: the engine joins relative_path to
  // mount_path verbatim.
  const file = { id: 'X1', name: 'someone-elses.txt', parents: ['OTHER'] }
  const { handler } = stub(deltaFor(file, [{ id: 'OTHER', name: 'elsewhere', parents: [] }]))
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { since_token: 't1' },
  })
  assert.deepEqual(out.items, [])
})

test("the mount's own root folder is never emitted as an item inside itself", () => {
  const file = { id: 'ROOT', name: 'the-mount', mimeType: 'application/vnd.google-apps.folder', parents: ['GRANDPARENT'] }
  const { handler } = stub([ok({ changes: [{ fileId: 'ROOT', file }], newStartPageToken: 't2' })])
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { since_token: 't1' },
  })
  assert.deepEqual(out.items, [])
})

test('a multi-parent file takes the FIRST parent chain that reaches the mount root', () => {
  // Drive is a DAG: a file created before September 2020 can sit in two folders.
  // The answer cannot be "correct" — the full walk lists the file under both and
  // keeps whichever it saw last — but it must be STABLE between runs, or the
  // node flip-flops exactly as the flat name made it.
  const file = { id: 'F2', name: 'shared.txt', parents: ['OUTSIDE', 'IN'] }
  const { handler } = stub(
    deltaFor(file, [
      { id: 'OUTSIDE', name: 'other', parents: [] },
      { id: 'IN', name: 'inside', parents: ['ROOT'] },
    ])
  )
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { since_token: 't1' },
  })
  assert.equal(out.items[0].relative_path, 'inside/shared.txt')
})

test('one folder chain is resolved once for a whole page of siblings', () => {
  const a = { id: 'F1', name: 'one.txt', parents: ['B'] }
  const b = { id: 'F2', name: 'two.txt', parents: ['B'] }
  const { handler, calls } = stub([
    ok({ changes: [{ fileId: 'F1', file: a }, { fileId: 'F2', file: b }], newStartPageToken: 't2' }),
    ok({ id: 'B', name: 'b', parents: ['ROOT'] }),
  ])
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { since_token: 't1' },
  })
  assert.deepEqual(
    out.items.map((i) => i.relative_path),
    ['b/one.txt', 'b/two.txt']
  )
  assert.equal(calls.length, 2, 'the parent lookup is cached across the page')
})

test('a deletion needs no path and is not subtree-filtered', () => {
  const { handler, calls } = stub([
    ok({ changes: [{ fileId: 'GONE', removed: true }], newStartPageToken: 't2' }),
  ])
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { since_token: 't1' },
  })
  assert.equal(out.items[0].type, 'deleted')
  assert.equal(out.items[0].item.external_id, 'GONE')
  assert.equal(calls.length, 1, 'a removed file has no metadata left to walk')
})

test('a whole-My-Drive mount resolves the "root" alias before walking parents', () => {
  // `parents` never contains the alias, so an unresolved "root" would make every
  // chain walk past the top and every item look like it lives outside the mount.
  const file = { id: 'F1', name: 'x.txt', parents: ['REALROOT'] }
  const { handler, calls } = stub([
    ok({ changes: [{ fileId: 'F1', file }], newStartPageToken: 't2' }),
    ok({ id: 'REALROOT' }),
  ])
  const out = handler({
    operation: 'get_changes',
    credential: CREDENTIAL,
    mount: { ...MOUNT, remote_root: '' },
    params: { since_token: 't1' },
  })
  assert.match(calls[1].url, /\/files\/root\?fields=id/)
  assert.equal(out.items[0].relative_path, 'x.txt')
})

// ---- create: the parameters the ENGINE actually sends ----------------------

test('a folder create uses the mapper mime type and the NODE\'s parent folder', () => {
  const { handler, calls } = stub([
    ok({ id: 'NEW', name: 'Gründung', mimeType: 'application/vnd.google-apps.folder', version: '1' }),
  ])
  const out = handler({
    operation: 'create',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: {
      payload: { name: 'Gründung', mimeType: 'application/vnd.google-apps.folder' },
      parent_id: 'ROOT',
      parent_external_id: 'SUB',
      relative_path: 'docs/Gründung',
    },
  })
  const body = calls[0].request.body
  assert.equal(body.name, 'Gründung')
  assert.equal(body.mimeType, 'application/vnd.google-apps.folder')
  assert.deepEqual(
    body.parents,
    ['SUB'],
    "the node's own parent wins over the mount root, or the folder is created at " +
      'the top of the mount and the next walk moves the local node to match'
  )
  assert.deepEqual(out, { external_id: 'NEW', etag: '1' })
})

test('a create with no name anywhere is refused before it reaches Drive', () => {
  const { handler, calls } = stub([])
  assert.throws(
    () =>
      handler({
        operation: 'create',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { payload: {}, parent_id: 'ROOT', relative_path: '' },
      }),
    (e) => e.code === 'config_error'
  )
  assert.equal(calls.length, 0)
})

test("a create falls back to relative_path's last segment for the name", () => {
  const { handler, calls } = stub([ok({ id: 'NEW', version: '1' })])
  handler({
    operation: 'create',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { payload: {}, parent_id: 'ROOT', relative_path: 'a/b/notes.md' },
  })
  assert.equal(calls[0].request.body.name, 'notes.md')
})

test("the engine's is_folder flag never reaches Drive as a field", () => {
  // Google rejects an unknown name in the resource body outright, so passing the
  // engine's own vocabulary through would 400 the create it describes.
  const { handler, calls } = stub([ok({ id: 'NEW', version: '1' })])
  handler({
    operation: 'create',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { payload: { name: 'f', is_folder: true }, parent_id: 'ROOT', relative_path: 'f' },
  })
  const body = calls[0].request.body
  assert.equal(body.mimeType, 'application/vnd.google-apps.folder')
  assert.equal(body.is_folder, undefined)
})

test('a create with an id-less 2xx fails loudly instead of adopting nothing', () => {
  const { handler } = stub([ok({ name: 'x' })])
  assert.throws(
    () =>
      handler({
        operation: 'create',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { payload: { name: 'x' }, parent_id: 'ROOT', relative_path: 'x' },
      }),
    /returned no file id/
  )
})

// ---- the byte channel ------------------------------------------------------

const CONTENT = { name: 'photo.jpg', mime_type: 'image/jpeg', size: 12, inline: true, content_base64: 'AAAA' }

test('a create that carries bytes answers with a resumable session for the ENGINE', () => {
  const { handler, calls } = stub([
    { status: 200, headers: { location: 'https://www.googleapis.com/upload/drive/v3/files?upload_id=S1' }, body: '' },
  ])
  const out = handler({
    operation: 'create',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: {
      payload: { name: 'photo.jpg' },
      parent_id: 'ROOT',
      parent_external_id: null,
      relative_path: 'photo.jpg',
      content: CONTENT,
    },
  })

  assert.match(calls[0].url, /\/upload\/drive\/v3\/files\?uploadType=resumable/)
  assert.match(calls[0].url, /fields=/, 'the session replays this query on its final response, which is where `version` comes from')
  assert.equal(calls[0].request.method, 'POST')
  assert.equal(calls[0].request.body.name, 'photo.jpg')
  assert.deepEqual(calls[0].request.body.parents, ['ROOT'])

  assert.equal(out.upload.url, 'https://www.googleapis.com/upload/drive/v3/files?upload_id=S1')
  assert.equal(out.upload.method, 'PUT')
  assert.equal(
    out.upload.chunk_size % (256 * 1024),
    0,
    'Drive rejects a non-final chunk that is not a multiple of 256 KiB, mid-transfer'
  )
  assert.deepEqual(
    out.upload.continue_statuses,
    [308],
    "Drive answers every non-final chunk with 308; an engine default of 2xx-only " +
      'would fail every multi-chunk upload on chunk one'
  )
  assert.equal(out.upload.headers, undefined, 'the session url is pre-authenticated')
})

test('a session with no Location header is a stated failure, not a silent success', () => {
  const { handler } = stub([{ status: 200, headers: {}, body: '' }])
  assert.throws(
    () =>
      handler({
        operation: 'create',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { payload: { name: 'p.jpg' }, parent_id: 'ROOT', relative_path: 'p.jpg', content: CONTENT },
      }),
    /no resumable session/
  )
})

test('an update that carries bytes sends the rename in the SAME session request', () => {
  const { handler, calls } = stub([
    ok({ version: '4' }), // the concurrency probe
    { status: 200, headers: { Location: 'https://up/S2' }, body: '' },
  ])
  const out = handler({
    operation: 'update',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { item_id: 'F1', etag: '4', payload: { name: 'renamed.jpg' }, content: CONTENT },
  })
  assert.match(calls[1].url, /\/upload\/drive\/v3\/files\/F1\?uploadType=resumable/)
  assert.equal(calls[1].request.method, 'PATCH')
  assert.equal(
    calls[1].request.body.name,
    'renamed.jpg',
    'a rename dropped here is still recorded as pushed, and the two names then ' +
      'diverge permanently'
  )
  assert.equal(out.upload.url, 'https://up/S2')
})

test('finalize_upload reads the id and the walk\'s own etag out of Drive\'s answer', () => {
  const { handler, calls } = stub([])
  const out = handler({
    operation: 'finalize_upload',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: {
      status: 200,
      body: { id: 'NEW', name: 'photo.jpg', version: '9', modifiedTime: '2026-08-30T00:00:00Z' },
      headers: { etag: '"ignored"' },
      intent: 'create',
      item_id: null,
    },
  })
  assert.deepEqual(out, { external_id: 'NEW', etag: '9' })
  assert.equal(calls.length, 0, 'the body already carries everything; no extra call')
})

test('finalize_upload reads the file back when the final response carries no version', () => {
  // A null etag falls back at the engine to the STALE pre-write value, and the
  // next walk then clobbers the bytes this upload just stored.
  const { handler, calls } = stub([ok({ id: 'NEW', name: 'photo.jpg', version: '3' })])
  const out = handler({
    operation: 'finalize_upload',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { status: 200, body: { id: 'NEW' }, headers: {}, intent: 'update', item_id: 'NEW' },
  })
  assert.equal(out.etag, '3')
  assert.match(calls[0].url, /\/files\/NEW\?fields=/)
})

test('finalize_upload refuses an id-less success', () => {
  const { handler } = stub([])
  assert.throws(
    () =>
      handler({
        operation: 'finalize_upload',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { status: 200, body: {}, headers: {}, intent: 'create', item_id: null },
      }),
    /returned no file id/
  )
})

test('finalize_upload keeps the shared taxonomy for a non-2xx final chunk', () => {
  const { handler } = stub([])
  assert.throws(
    () =>
      handler({
        operation: 'finalize_upload',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { status: 401, body: {}, headers: {}, intent: 'create', item_id: null },
      }),
    (e) => e.code === 'auth_expired'
  )
})

// ---- metadata update -------------------------------------------------------

test('a metadata-only update PATCHes the mapper payload and returns the new version', () => {
  const { handler, calls } = stub([
    ok({ version: '4' }),
    ok({ id: 'F1', name: 'renamed.txt', version: '5', modifiedTime: '2026-08-30T00:00:00Z' }),
  ])
  const out = handler({
    operation: 'update',
    credential: CREDENTIAL,
    mount: MOUNT,
    params: { item_id: 'F1', etag: '4', payload: { name: 'renamed.txt' }, fields: ['title'] },
  })
  assert.equal(calls[1].request.method, 'PATCH')
  assert.deepEqual(calls[1].request.body, { name: 'renamed.txt' })
  assert.deepEqual(
    out,
    { external_id: 'F1', etag: '5' },
    'the receipt must carry the etag the NEXT walk computes, or that walk ' +
      'rebuilds the node from remote and reverts this push'
  )
})

test('an update with no item_id is refused before any request', () => {
  const { handler, calls } = stub([])
  assert.throws(
    () => handler({ operation: 'update', credential: CREDENTIAL, mount: MOUNT, params: { payload: { name: 'x' } } }),
    (e) => e.code === 'config_error'
  )
  assert.equal(calls.length, 0)
})

// ---- capabilities ----------------------------------------------------------

test('the declared write surface matches what is implemented', () => {
  const { handler } = stub([])
  const caps = handler({ operation: 'capabilities', credential: CREDENTIAL, mount: MOUNT, params: {} })
  assert.equal(caps.can_create, true)
  assert.equal(caps.can_update, true)
  assert.equal(caps.can_delete, true)
  assert.equal(caps.can_create_folders, true)
  assert.equal(
    caps.accepts_content,
    true,
    'without it the engine sends metadata only and a mirrored file arrives at ' +
      'Drive as a name with no content'
  )
  assert.equal(caps.supports_trash, true)
  assert.equal(caps.default_delete_policy, 'detach')
  assert.deepEqual(caps.mutable_fields, ['title'])
})

test('a 403 on a write names the missing scope instead of retrying forever', () => {
  const { handler } = stub([
    { status: 403, headers: {}, body: { error: { errors: [{ reason: 'insufficientPermissions' }], message: 'Insufficient Permission' } } },
  ])
  assert.throws(
    () =>
      handler({
        operation: 'create',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { payload: { name: 'x' }, parent_id: 'ROOT', relative_path: 'x' },
      }),
    (e) => e.code === 'config_error' && /RECONNECT/.test(e.message)
  )
})

test('a 403 that is a quota answer stays rate_limited', () => {
  const { handler } = stub([
    { status: 403, headers: {}, body: { error: { errors: [{ reason: 'userRateLimitExceeded' }] } } },
  ])
  assert.throws(
    () =>
      handler({
        operation: 'create',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { payload: { name: 'x' }, parent_id: 'ROOT', relative_path: 'x' },
      }),
    (e) => e.code === 'rate_limited'
  )
})

// ---- delete: the third declared write, and the least exercised -------------

test('a 403 on a DELETE names the missing scope, exactly as create and update do', () => {
  // The gap this pins: `delete` and `delete(trash)` were the only writes that
  // did not tell `raiseForStatus` they were writes, so a missing write scope —
  // the first thing a newly writable mount hits — came back as a plain Error.
  // A plain Error is `Transient`, i.e. the identical doomed request re-sent on
  // every drain forever, with nothing anywhere naming the scope.
  const { handler } = stub([
    ok({ version: '4' }), // checkVersion's read succeeds: reads are in scope
    { status: 403, headers: {}, body: { error: { errors: [{ reason: 'insufficientPermissions' }] } } },
  ])
  assert.throws(
    () =>
      handler({
        operation: 'delete',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { item_id: 'F1', policy: 'purge', etag: '4' },
      }),
    (e) => e.code === 'config_error' && /RECONNECT/.test(e.message)
  )
})

test('a 403 on a TRASH delete is named too', () => {
  const { handler } = stub([
    ok({ version: '4' }),
    { status: 403, headers: {}, body: { error: { errors: [{ reason: 'insufficientPermissions' }] } } },
  ])
  assert.throws(
    () =>
      handler({
        operation: 'delete',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { item_id: 'F1', policy: 'trash', etag: '4' },
      }),
    (e) => e.code === 'config_error'
  )
})

test('trash and purge are different requests, because supports_trash promises they are', () => {
  const trash = stub([ok({ version: '4' }), ok({ id: 'F1' })])
  assert.deepEqual(
    trash.handler({
      operation: 'delete',
      credential: CREDENTIAL,
      mount: MOUNT,
      params: { item_id: 'F1', policy: 'trash', etag: '4' },
    }),
    { deleted: true, trashed: true }
  )
  assert.equal(trash.calls[1].request.method, 'PATCH')
  assert.deepEqual(trash.calls[1].request.body, { trashed: true })

  const purge = stub([ok({ version: '4' }), ok({})])
  assert.deepEqual(
    purge.handler({
      operation: 'delete',
      credential: CREDENTIAL,
      mount: MOUNT,
      params: { item_id: 'F1', policy: 'purge', etag: '4' },
    }),
    { deleted: true }
  )
  assert.equal(purge.calls[1].request.method, 'DELETE')
})

test('a delete whose remote moved since the last sync is a conflict, not a deletion', () => {
  // The engine sends the pre-image's etag on every delete; honouring it is the
  // whole difference between "delete the thing the operator saw" and "delete
  // whatever is there now".
  const { handler, calls } = stub([ok({ version: '9' })])
  assert.throws(
    () =>
      handler({
        operation: 'delete',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { item_id: 'F1', policy: 'purge', etag: '4' },
      }),
    (e) => e.code === 'conflict'
  )
  assert.equal(calls.length, 1, 'nothing may be deleted after a version mismatch')
})

test('an already-gone file deletes idempotently under either policy', () => {
  for (const policy of ['purge', 'trash']) {
    const { handler, calls } = stub([{ status: 404, headers: {}, body: {} }])
    assert.equal(
      handler({
        operation: 'delete',
        credential: CREDENTIAL,
        mount: MOUNT,
        params: { item_id: 'F1', policy: policy, etag: '4' },
      }).deleted,
      true
    )
    assert.equal(calls.length, 1, 'a 404 from the version probe settles it')
  }
})
