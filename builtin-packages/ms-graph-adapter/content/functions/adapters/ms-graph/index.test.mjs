// SPDX-License-Identifier: BSL-1.1
//
// Adapter-level tests. Run with `node --test index.test.mjs`.
//
// Every case here is a production incident, not a hypothetical:
//
//  * `folder_id` ignored on a drive `list` — nested content silently never
//    imported, and the walk never converged. A flat drive masks it completely,
//    which is why it survived so long: the FIRST list call is the root.
//  * `has_more` absent — the delta loop had only "the token stopped changing"
//    to stop on, and Graph mints a fresh delta token on every poll of an idle
//    feed, so it never stopped until the job watchdog killed the run.
//  * `Retry-After` discarded — the engine guessed a backoff against a provider
//    that had stated one, which is how throttling becomes self-sustaining.
//  * default (mutable) message ids — moving a mail between folders reads as
//    delete + create, destroying the node with its attachments and history.
//  * a drive write that carries bytes through `body` rather than `bodyBase64` —
//    the host would send the BASE64 TEXT of the file and the provider would
//    answer 201, so every uploaded document lands corrupt and reports success.

import test from 'node:test'
import assert from 'node:assert/strict'

import { opList, opGetContent } from './read.js'
import { opGetChanges } from './changes.js'
import { raiseForStatus } from './http.js'
import { outlookHeaders, useImmutableIds } from './mount.js'
import { opCreate, opDelete, opUpdate } from './write.js'
import { opFinalizeUpload, SIMPLE_PUT_MAX, UPLOAD_CHUNK_SIZE } from './drive-upload.js'
import { opCapabilities } from './capabilities.js'
import { toExternalItem } from './items.js'

// ---- host stub -------------------------------------------------------------

/** Record every request the adapter makes and answer from a queue. */
function stubHttp(responses) {
  const calls = []
  globalThis.raisin = {
    http: {
      fetch(url, request) {
        calls.push({ url, request })
        const next = responses.shift()
        if (!next) throw new Error(`unexpected extra request: ${url}`)
        return { status: 200, headers: {}, body: {}, ...next }
      },
    },
  }
  return calls
}

const CREDENTIAL = { access_token: 'tok' }

function mailMount(extra) {
  return { sync_config: { resource: 'mail' }, ...extra }
}
function filesMount(extra) {
  return { sync_config: { resource: 'files' }, remote_root: 'ROOT', ...extra }
}

// ---- list: folder_id ------------------------------------------------------

test('a drive list asks for the folder the engine named, not the mount root', () => {
  const calls = stubHttp([{ body: { value: [] } }])
  opList(CREDENTIAL, filesMount(), { folder_id: 'SUBFOLDER-1' })

  assert.match(
    calls[0].url,
    /\/items\/SUBFOLDER-1\/children/,
    'the engine recurses folders explicitly; ignoring folder_id re-lists the ' +
      'root forever and nested content is never imported'
  )
  assert.doesNotMatch(calls[0].url, /\/items\/ROOT\/children/)
})

test('a drive list with no folder_id lists the mount root', () => {
  const calls = stubHttp([{ body: { value: [] } }])
  opList(CREDENTIAL, filesMount(), {})
  assert.match(calls[0].url, /\/items\/ROOT\/children/)
})

// ---- get_changes: has_more ------------------------------------------------

// Mint a cursor the way the engine gets one: from a first, token-less call. A
// bare Graph URL is no longer a usable `since_token` — see the identity tests
// below — so anything that resumes must resume from a real one.
function mintCursor(mount, link) {
  stubHttp([{ body: { value: [], '@odata.deltaLink': link || 'https://graph/seed' } }])
  return opGetChanges(CREDENTIAL, mount, {}).next_token
}

test('has_more distinguishes a mid-enumeration page from a caught-up cursor', () => {
  const token = mintCursor(mailMount())

  // A nextLink means "more pages right now".
  stubHttp([{ body: { value: [], '@odata.nextLink': 'https://graph/next' } }])
  const paging = opGetChanges(CREDENTIAL, mailMount(), { since_token: token })
  assert.equal(paging.has_more, true)
  assert.match(paging.next_token, /graph\/next/)

  // A deltaLink means "caught up — this token is the NEXT RUN's resume point".
  // Reporting has_more here is what spun the loop against an idle feed.
  stubHttp([{ body: { value: [], '@odata.deltaLink': 'https://graph/delta' } }])
  const caught = opGetChanges(CREDENTIAL, mailMount(), { since_token: token })
  assert.equal(caught.has_more, false)
  assert.match(caught.next_token, /graph\/delta/)
})

test('get_changes never returns a null cursor', () => {
  // Null reads to the engine as "no resumable cursor exists" — the stored
  // cursor must survive a page that carries neither link.
  const token = mintCursor(mailMount(), 'https://graph/keep-me')
  stubHttp([{ body: { value: [] } }])
  const out = opGetChanges(CREDENTIAL, mailMount(), { since_token: token })
  assert.match(out.next_token, /graph\/keep-me/)
  assert.equal(out.has_more, false)
})

// ---- get_changes: cursor identity -----------------------------------------
//
// Graph bakes the query INTO the delta link: $select and the calendarView date
// range are frozen at mint time and replayed on every later poll. A stored token
// therefore encodes configuration that may since have changed, and the engine —
// which treats it as opaque — cannot notice. Two data-loss bugs came from
// exactly this: turning on `include_body` was a permanent no-op, and a calendar
// window never slid forward, so meetings past `days_ahead` stopped arriving.

test('widening the projection invalidates a cursor minted for the old one', () => {
  const narrow = mintCursor(mailMount())

  // Same mount, same projection: the cursor is accepted.
  stubHttp([{ body: { value: [], '@odata.deltaLink': 'https://graph/d2' } }])
  assert.doesNotThrow(() => opGetChanges(CREDENTIAL, mailMount(), { since_token: narrow }))

  // include_body widens $select, which the stored link cannot carry.
  const wide = { sync_config: { resource: 'mail', include_body: true } }
  stubHttp([{ body: { value: [] } }])
  assert.throws(
    () => opGetChanges(CREDENTIAL, wide, { since_token: narrow }),
    (e) => e.code === 'cursor_invalid' && /different query/.test(e.message)
  )
})

test('a cursor minted before identity existed is resynced once, not trusted', () => {
  // A bare Graph URL is what every pre-upgrade mount has stored. It cannot be
  // checked against the current projection precisely because nothing recorded
  // what minted it, so it is discarded rather than grandfathered — the two bugs
  // above are both silent, and a grandfathered token carries them forward
  // indefinitely.
  stubHttp([{ body: { value: [] } }])
  assert.throws(
    () => opGetChanges(CREDENTIAL, mailMount(), { since_token: 'https://graph/legacy' }),
    (e) => e.code === 'cursor_invalid' && /predates cursor identity/.test(e.message)
  )
})

// ---- get_changes: partial payloads ----------------------------------------

test('a flag-only delta entry is re-read in full, not mapped as nulls', () => {
  // Marking a message unread in Outlook produces a delta entry carrying ONLY
  // the changed property. `mailMeta` answers a missing key with an explicit
  // null — correct for the full walk, where absent really does mean no value —
  // and the engine's upsert rebuilds the property map wholesale from it. So one
  // click in Outlook wiped the message's subject, sender, recipients and date to
  // null while the node kept its id and its etag, leaving nothing downstream
  // able to tell "has no subject" from "we were not told the subject".
  const token = mintCursor(mailMount())
  const calls = stubHttp([
    {
      body: {
        value: [{ '@odata.etag': 'W/"2"', id: 'MSG-1', isRead: false }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
    // The re-read the adapter must perform before anything maps it.
    {
      body: {
        id: 'MSG-1',
        subject: 'Invoice 4711',
        isRead: false,
        receivedDateTime: '2026-08-12T09:00:00Z',
        from: { emailAddress: { name: 'Ada', address: 'ada@example.com' } },
      },
    },
  ])

  const out = opGetChanges(CREDENTIAL, mailMount(), { since_token: token })

  assert.match(calls[1].url, /\/messages\/MSG-1/, 'the partial entry must be re-read')
  assert.match(calls[1].url, /\$select=/, 'and re-read with the mount projection')

  const meta = out.items[0].item.metadata
  assert.equal(meta.subject, 'Invoice 4711')
  assert.equal(meta.from_address, 'ada@example.com')
  assert.equal(meta.date, '2026-08-12T09:00:00Z')
  assert.equal(meta.unread, true, 'and the change itself still lands')
})

test('a complete delta entry costs no extra request', () => {
  // The re-read is per CHANGED-AND-PARTIAL item. New mail arrives complete and
  // must not pay for it.
  const token = mintCursor(mailMount())
  const calls = stubHttp([
    {
      body: {
        value: [{ id: 'MSG-2', subject: 'Hello', receivedDateTime: '2026-08-12T10:00:00Z' }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const out = opGetChanges(CREDENTIAL, mailMount(), { since_token: token })
  assert.equal(calls.length, 1, 'one request, not two')
  assert.equal(out.items[0].item.metadata.subject, 'Hello')
})

// ---- get_changes: drive deletions -----------------------------------------

test('a driveItem deletion is a delete, not an update', () => {
  // TWO removal vocabularies. Outlook marks a deletion with `@removed`; a
  // driveItem marks it with a `deleted` FACET and no annotation, so testing only
  // for `@removed` meant every OneDrive and SharePoint deletion arrived as an
  // ordinary update and the file persisted in the workspace indefinitely.
  const token = mintCursor(filesMount())
  stubHttp([
    {
      body: {
        value: [
          { id: 'GONE', name: 'old.txt', file: {}, deleted: { state: 'deleted' } },
          { id: 'LIVE', name: 'new.txt', file: {} },
        ],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
    // A live drive item makes the page resolve the mount root once, so paths
    // can be made relative to it.
    { body: { id: 'ROOT', name: 'Mounted', parentReference: { path: '/drive/root:' } } },
  ])
  const out = opGetChanges(CREDENTIAL, filesMount(), { since_token: token })
  const kinds = out.items.map((c) => c.type)
  assert.deepEqual(kinds, ['deleted', 'updated'])
  assert.equal(out.items[0].item.external_id, 'GONE')
})

// ---- shared/shortcut drive items ------------------------------------------

test('a shared folder shortcut is a folder, not a zero-byte file', () => {
  // "Add to my OneDrive" returns a shortcut whose `folder` facet is nested under
  // `remoteItem`. Reading only the top level stored it as an empty leaf file and
  // its entire subtree was never walked.
  const item = toExternalItem(
    {
      id: 'SHORTCUT',
      name: 'Team Docs',
      remoteItem: {
        id: 'REMOTE-1',
        folder: { childCount: 12 },
        parentReference: { driveId: 'DRIVE-B' },
      },
    },
    'files',
    filesMount()
  )
  assert.equal(item.is_folder, true, 'a shortcut to a folder IS a folder')
  assert.equal(item.metadata.remote_drive_id, 'DRIVE-B', 'recursing needs the real drive')
  assert.equal(item.metadata.remote_item_id, 'REMOTE-1')
})

// ---- error taxonomy -------------------------------------------------------

test('a stated Retry-After is carried to the engine, and a missing one is not invented', () => {
  assert.throws(
    () => raiseForStatus({ status: 429, headers: { 'Retry-After': '120' }, body: {} }, 'list'),
    (e) => e.code === 'rate_limited' && e.retry_after === 120 && /retry_after=120/.test(e.message)
  )
  // Header casing is not guaranteed across hosts.
  assert.throws(
    () => raiseForStatus({ status: 503, headers: { 'retry-after': '30' }, body: {} }, 'list'),
    (e) => e.code === 'rate_limited' && e.retry_after === 30
  )
  // Silence means "the provider did not say" — the engine's own backoff
  // applies. Inventing a number here would be a guess wearing an instruction's
  // clothes.
  assert.throws(
    () => raiseForStatus({ status: 429, headers: {}, body: {} }, 'list'),
    (e) => e.code === 'rate_limited' && e.retry_after === undefined
  )
  // An HTTP-date form is ignored rather than mis-parsed as NaN.
  assert.throws(
    () =>
      raiseForStatus(
        { status: 429, headers: { 'Retry-After': 'Wed, 21 Oct 2026 07:28:00 GMT' }, body: {} },
        'list'
      ),
    (e) => e.code === 'rate_limited' && e.retry_after === undefined
  )
})

test('status codes map to the reserved engine codes', () => {
  const cases = [
    [401, 'auth_expired'],
    [403, 'auth_expired'],
    [429, 'rate_limited'],
    [503, 'rate_limited'],
    [504, 'rate_limited'],
    [400, 'config_error'],
    [404, 'config_error'],
    [410, 'cursor_invalid'],
  ]
  for (const [status, code] of cases) {
    assert.throws(
      () => raiseForStatus({ status, headers: {}, body: {} }, 'ctx'),
      (e) => e.code === code,
      `HTTP ${status} must map to ${code}`
    )
  }
  // Graph reports a stale delta cursor as 400 + syncStateNotFound as often as
  // 410, and reading that as config_error badges a healthy mount broken.
  assert.throws(
    () =>
      raiseForStatus(
        { status: 400, headers: {}, body: { error: { code: 'syncStateNotFound', message: 'x' } } },
        'get_changes'
      ),
    (e) => e.code === 'cursor_invalid'
  )
  // 408/409 stay retryable — deliberately NOT in the config_error bucket.
  assert.throws(
    () => raiseForStatus({ status: 408, headers: {}, body: {} }, 'ctx'),
    (e) => e.code === undefined
  )
  // Success is not an error.
  assert.equal(raiseForStatus({ status: 200, headers: {}, body: {} }, 'ctx'), undefined)
})

// ---- file content ---------------------------------------------------------

test('drive content is POINTED at with a freshly minted url, never a stored one', () => {
  // The adapter cannot carry the bytes: the host decodes every response as
  // text, so a PDF through a JS string comes back corrupted. It answers with a
  // url and the ENGINE downloads it in Rust.
  const calls = stubHttp([
    {
      body: {
        id: '01ABC',
        file: { mimeType: 'application/pdf' },
        '@microsoft.graph.downloadUrl': 'https://dl.example/fresh-link',
      },
    },
  ])
  const out = opGetContent(CREDENTIAL, filesMount(), { item_id: '01ABC' })

  assert.equal(out.fetch_url, 'https://dl.example/fresh-link')
  assert.equal(out.mime_type, 'application/pdf')
  // Never inline bytes — that is the corruption path.
  assert.equal(out.content, undefined)
  assert.equal(out.content_base64, undefined)
  // The url is minted on THIS call. A pre-authenticated Graph link lives about
  // an hour, so serving a copy captured at sync time is the bug this avoids.
  assert.match(calls[0].url, /\/items\/01ABC\?\$select=/)
  assert.match(calls[0].url, /downloadUrl/)
})

test('a drive item with no download url is refused rather than half-served', () => {
  stubHttp([{ body: { id: 'FOLDER-1', folder: { childCount: 3 } } }])
  assert.throws(
    () => opGetContent(CREDENTIAL, filesMount(), { item_id: 'FOLDER-1' }),
    (e) => e.code === 'config_error',
    'a folder has no content; storing an empty file that reads as "fetched" is worse'
  )
  // A vanished item is null (settled), not an error.
  stubHttp([{ status: 404, body: {} }])
  assert.equal(opGetContent(CREDENTIAL, filesMount(), { item_id: 'GONE' }), null)
})

// ---- update receipts ------------------------------------------------------
//
// The receipt's etag must be THE ONE THE NEXT WALK/DELTA COMPUTES for the
// post-write state. A receipt on any other representation makes the run after
// a push mismatch its own write: the node is rebuilt wholesale from remote and
// __pushed_state reseeded, silently reverting edits made while the run was in
// flight — the read path has no local-wins branch. (The Hue adapter shipped
// exactly this and had to fix it with a read-after-write.)

test('the update receipt etag is the one the next walk computes', () => {
  // Graph answered the PATCH with the full updated message: the receipt reads
  // it with the walk's own formula (@odata.etag || eTag || lastModifiedDateTime),
  // so it must equal toExternalItem's etag for the identical body.
  const updated = {
    id: 'MSG-1',
    '@odata.etag': 'W/"CQAAABYAAABn"',
    lastModifiedDateTime: '2026-08-12T10:00:00Z',
    subject: 'x',
    isRead: true,
  }
  const calls = stubHttp([{ body: updated }])
  const receipt = opUpdate(CREDENTIAL, mailMount(), {
    item_id: 'MSG-1',
    payload: { isRead: true },
  })
  assert.equal(receipt.external_id, 'MSG-1')
  assert.equal(receipt.etag, toExternalItem(updated, 'mail', mailMount()).etag)
  // The PATCH body sufficed — no read-after-write request was spent.
  assert.equal(calls.length, 1)
})

test('a mail item on the ISO fallback still gets a walk-identical receipt', () => {
  // Mail items sometimes carry no @odata.etag at all; the read paths then fall
  // back to lastModifiedDateTime. The receipt must fall back the SAME way — a
  // null here would leave the engine holding the STALE pre-write etag, and the
  // next delta would rebuild the node from the push's own echo.
  const updated = { id: 'MSG-2', lastModifiedDateTime: '2026-08-12T10:05:00Z' }
  stubHttp([{ body: updated }])
  const receipt = opUpdate(CREDENTIAL, mailMount(), {
    item_id: 'MSG-2',
    payload: { isRead: false },
  })
  assert.equal(receipt.etag, toExternalItem(updated, 'mail', mailMount()).etag)
})

test('a bodiless PATCH answer triggers a read-after-write, not a null etag', () => {
  const fresh = { id: 'MSG-3', '@odata.etag': 'W/"AFTER"', subject: 'x' }
  const calls = stubHttp([
    { status: 200, body: {} }, // the PATCH echoed nothing usable
    { body: fresh }, // the read-back
  ])
  const receipt = opUpdate(CREDENTIAL, mailMount(), {
    item_id: 'MSG-3',
    payload: { isRead: true },
  })
  // The read-back goes through opGet — same $select, same toExternalItem — so
  // the stamped etag is byte-identical to what the next walk computes, with
  // strict item-build parity between the single get and the full walk.
  assert.match(calls[1].url, /\/messages\/MSG-3\?\$select=/)
  assert.equal(receipt.external_id, 'MSG-3')
  assert.equal(receipt.etag, toExternalItem(fresh, 'mail', mailMount()).etag)
})

// ---- immutable ids --------------------------------------------------------

test('Outlook reads and writes share one id space, on by default', () => {
  // Default ON: with mutable ids, filing a mail into a folder changes its id,
  // so the delta reports delete + create and the node — with its attachment
  // subnodes and history — is destroyed and rebuilt.
  assert.equal(useImmutableIds(mailMount()), true)
  assert.equal(outlookHeaders(mailMount())['Prefer'], 'IdType="ImmutableId"')
  assert.equal(useImmutableIds({ sync_config: { resource: 'calendar' } }), true)

  // The list request actually carries it.
  const calls = stubHttp([{ body: { value: [] } }])
  opList(CREDENTIAL, mailMount(), {})
  assert.equal(calls[0].request.headers['Prefer'], 'IdType="ImmutableId"')

  // Opt-out per mount, for a mount deferring the one-time re-import.
  const off = mailMount({ sync_config: { resource: 'mail', immutable_ids: false } })
  assert.equal(useImmutableIds(off), false)
  assert.equal(outlookHeaders(off)['Prefer'], undefined)

  // Drives have no immutable-id notion — no noise on the busiest resource.
  assert.equal(useImmutableIds(filesMount()), false)
  assert.equal(outlookHeaders(filesMount())['Prefer'], undefined)

  // Caller headers survive the merge: a PATCH keeps its If-Match/Content-Type.
  const merged = outlookHeaders(mailMount(), { 'If-Match': 'W/"1"' })
  assert.equal(merged['If-Match'], 'W/"1"')
  assert.equal(merged['Prefer'], 'IdType="ImmutableId"')
})

// ---- the drive write path -------------------------------------------------
//
// A drive write is the only one in this adapter that carries BYTES, and every
// case below is a way that goes silently wrong: the base64 sent as text, a file
// Graph will not take in one request sent as one anyway, and an upload that
// reports success with nothing the engine can match the file to.

/** The engine's `content` for a file small enough to pass inline. */
function inlineContent(extra) {
  return {
    name: 'report.pdf',
    mime_type: 'application/pdf',
    size: 11,
    content_base64: 'aGVsbG8gd29ybGQ=',
    inline: true,
    ...extra,
  }
}

const FILE_PAYLOAD = { name: 'report.pdf', '@microsoft.graph.conflictBehavior': 'rename' }

test('a small create PUTs the BYTES and returns an adoptable id', () => {
  const calls = stubHttp([
    { status: 201, body: { id: '01NEW', eTag: '"{GUID},1"', name: 'report.pdf' } },
  ])
  const receipt = opCreate(CREDENTIAL, filesMount(), {
    payload: FILE_PAYLOAD,
    parent_id: 'FOLDER-1',
    content: inlineContent(),
  })

  assert.equal(calls[0].request.method, 'PUT')
  assert.match(calls[0].url, /\/items\/FOLDER-1:\/report\.pdf:\/content/)
  // THE bug this file exists for: `body` would transmit the base64 TEXT.
  assert.equal(calls[0].request.bodyBase64, 'aGVsbG8gd29ybGQ=')
  assert.equal(calls[0].request.body, undefined)
  assert.equal(calls[0].request.headers['Content-Type'], 'application/pdf')

  assert.equal(receipt.external_id, '01NEW')
  // The walk's own formula, so the next run does not mismatch its own write.
  assert.equal(
    receipt.etag,
    toExternalItem({ id: '01NEW', eTag: '"{GUID},1"' }, 'files', filesMount()).etag
  )
  assert.equal(receipt.upload, undefined)
})

test('a create with no parent_id writes into the mount root', () => {
  const calls = stubHttp([{ status: 201, body: { id: '01NEW', eTag: '"e"' } }])
  opCreate(CREDENTIAL, filesMount(), { payload: FILE_PAYLOAD, content: inlineContent() })
  assert.match(calls[0].url, /\/items\/ROOT:\/report\.pdf:\/content/)
})

test('an oversized create answers with an upload session and NO external_id', () => {
  // Decided from `content.size`, never from whether base64 arrived: the engine
  // inlines up to 8 MiB and Microsoft's simple PUT stops at 4, so bytes can be
  // present and still have to go through a session.
  const big = inlineContent({ size: SIMPLE_PUT_MAX + 1 })
  const calls = stubHttp([{ body: { uploadUrl: 'https://contoso.sharepoint.com/_api/upl' } }])
  const out = opCreate(CREDENTIAL, filesMount(), {
    payload: FILE_PAYLOAD,
    parent_id: 'FOLDER-1',
    content: big,
  })

  assert.match(calls[0].url, /:\/report\.pdf:\/createUploadSession$/)
  assert.equal(calls[0].request.method, 'POST')
  // An ordinary JSON POST to Graph — no bytes, no new host in allowed_urls.
  assert.equal(calls[0].request.bodyBase64, undefined)
  assert.equal(calls[0].request.body.item['@microsoft.graph.conflictBehavior'], 'rename')

  assert.equal(out.upload.url, 'https://contoso.sharepoint.com/_api/upl')
  assert.equal(out.upload.chunk_size % (320 * 1024), 0, 'Graph rejects a non-multiple mid-transfer')
  assert.equal(out.upload.chunk_size, UPLOAD_CHUNK_SIZE)
  // Nothing to adopt yet. An external_id here would make the engine adopt a
  // node for a file that does not exist.
  assert.equal(out.external_id, undefined)
  // The session URL is pre-authenticated; forwarding our bearer token would
  // hand a Graph credential to a host we do not otherwise talk to.
  assert.equal(out.upload.headers, undefined)
})

test('an unknown size takes the session path rather than guessing small', () => {
  // Guessing "small" costs a 413 no retry can turn into a success.
  const calls = stubHttp([{ body: { uploadUrl: 'https://contoso.sharepoint.com/u' } }])
  const out = opCreate(CREDENTIAL, filesMount(), {
    payload: FILE_PAYLOAD,
    content: { name: 'report.pdf', inline: false },
  })
  assert.match(calls[0].url, /createUploadSession$/)
  assert.ok(out.upload.url)
})

test('inline bytes that never arrived are refused, not stored as an empty file', () => {
  // Graph would answer 201 with a real id and the engine would adopt a
  // zero-byte file as successfully mirrored.
  stubHttp([])
  assert.throws(
    () =>
      opCreate(CREDENTIAL, filesMount(), {
        payload: FILE_PAYLOAD,
        content: { name: 'report.pdf', size: 11, inline: true },
      }),
    (e) => e.code === 'config_error' && /content_base64/.test(e.message)
  )
})

test('a metadata-only update is a plain PATCH and attempts no upload', () => {
  const calls = stubHttp([
    { body: { id: '01ABC', eTag: '"{GUID},2"', name: 'renamed.pdf' } },
  ])
  const receipt = opUpdate(CREDENTIAL, filesMount(), {
    item_id: '01ABC',
    payload: { name: 'renamed.pdf' },
    etag: '"{GUID},1"',
  })
  assert.equal(calls.length, 1, 'one request — no content PUT, no session')
  assert.equal(calls[0].request.method, 'PATCH')
  assert.match(calls[0].url, /\/items\/01ABC$/)
  assert.doesNotMatch(calls[0].url, /content|createUploadSession/)
  assert.equal(calls[0].request.headers['If-Match'], '"{GUID},1"')
  assert.equal(receipt.external_id, '01ABC')
})

test('an update with bytes writes the content, and applies a rename first', () => {
  // A content PUT addresses the item by id and cannot rename it. Dropping the
  // rename silently would still be baselined as pushed, and the two names then
  // diverge permanently.
  const calls = stubHttp([
    { body: { id: '01ABC', eTag: '"{GUID},2"' } }, // the rename PATCH
    { body: { id: '01ABC', eTag: '"{GUID},3"' } }, // the content PUT
  ])
  const receipt = opUpdate(CREDENTIAL, filesMount(), {
    item_id: '01ABC',
    payload: { name: 'renamed.pdf' },
    content: inlineContent(),
  })
  assert.equal(calls[0].request.method, 'PATCH')
  assert.equal(calls[1].request.method, 'PUT')
  assert.match(calls[1].url, /\/items\/01ABC\/content$/)
  assert.equal(calls[1].request.bodyBase64, 'aGVsbG8gd29ybGQ=')
  // The receipt is the LAST write's state, not the rename's.
  assert.equal(receipt.etag, '"{GUID},3"')
})

test('finalize_upload parses the id and the etag out of the session response', () => {
  const receipt = opFinalizeUpload(filesMount(), {
    status: 201,
    intent: 'create',
    body: { id: '01BIG', '@odata.etag': 'W/"7"', name: 'movie.mp4' },
  })
  assert.equal(receipt.external_id, '01BIG')
  assert.equal(receipt.etag, 'W/"7"')
})

test('finalize_upload throws when the final response carries no id', () => {
  // An upload that reports success without an id makes the engine adopt a node
  // it cannot match — undeletable, and duplicated by the next reconcile.
  assert.throws(
    () => opFinalizeUpload(filesMount(), { status: 200, intent: 'create', body: {} }),
    (e) => /no driveItem id/.test(e.message)
  )
  // 202 means Graph is still waiting for the next fragment.
  assert.throws(
    () => opFinalizeUpload(filesMount(), { status: 202, body: { id: 'X' } }),
    (e) => e.code === 'config_error' && /not finished/.test(e.message)
  )
  // A failure keeps the shared taxonomy rather than growing a second one.
  assert.throws(
    () => opFinalizeUpload(filesMount(), { status: 401, body: {} }),
    (e) => e.code === 'auth_expired'
  )
})

test('a drive delete trashes, and refuses to pretend it can purge', () => {
  const calls = stubHttp([{ status: 204, body: {} }])
  const out = opDelete(CREDENTIAL, filesMount(), { item_id: '01ABC', policy: 'trash' })
  assert.equal(calls[0].request.method, 'DELETE')
  assert.deepEqual(out, { external_id: '01ABC', deleted: true })

  // Graph v1.0 has no permanent delete for a driveItem. Answering "destroyed"
  // to an operator who typed the one policy nothing defaults to is the failure
  // that matters here.
  stubHttp([])
  assert.throws(
    () => opDelete(CREDENTIAL, filesMount(), { item_id: '01ABC', policy: 'purge' }),
    (e) => e.code === 'config_error' && /no permanent delete/.test(e.message)
  )

  // Already gone is SUCCESS — the desired end state is already true.
  stubHttp([{ status: 404, body: {} }])
  assert.deepEqual(opDelete(CREDENTIAL, filesMount(), { item_id: 'GONE' }), {
    external_id: 'GONE',
    deleted: true,
  })
})

test('a 403 on a drive write names the FILES scope, not the mail one', () => {
  // 403 on a write is almost never a stale token; it is the write scope the
  // connector never requested.
  stubHttp([{ status: 403, body: { error: { code: 'accessDenied' } } }])
  assert.throws(
    () =>
      opUpdate(CREDENTIAL, filesMount(), { item_id: '01ABC', payload: { name: 'x' } }),
    (e) => e.code === 'config_error' && /Files\.ReadWrite/.test(e.message)
  )
})

// ---- capabilities: what the write flags promise ---------------------------

test('files declares the mirror set and the byte channel; mail and calendar are unchanged', () => {
  const files = opCapabilities(filesMount())
  assert.equal(files.can_write, true)
  assert.equal(files.can_create, true)
  assert.equal(files.can_update, true)
  assert.equal(files.can_delete, true)
  // Without this the engine sends metadata only, and a "mirrored" file arrives
  // at OneDrive as a name with no content.
  assert.equal(files.accepts_content, true)
  assert.equal(files.supports_trash, true)
  assert.equal(files.default_delete_policy, 'trash')
  // Declared only for what is implemented: no folder create, no command surface.
  assert.equal(files.can_create_folders, false)
  assert.equal(files.can_submit, undefined)

  // Mail keeps update + submit and nothing else; the drive work must not have
  // widened it.
  const mail = opCapabilities(mailMount())
  assert.equal(mail.can_update, true)
  assert.equal(mail.can_submit, true)
  assert.equal(mail.can_create, undefined)
  assert.equal(mail.can_delete, undefined)
  assert.equal(mail.accepts_content, undefined)
  assert.deepEqual(mail.mutable_fields, ['unread', 'is_read'])

  // Calendar keeps the full mirror set and carries NO byte channel.
  const cal = opCapabilities({ sync_config: { resource: 'calendar' } })
  assert.equal(cal.can_create, true)
  assert.equal(cal.can_delete, true)
  assert.equal(cal.can_submit, true)
  assert.equal(cal.accepts_content, undefined)
  assert.equal(cal.default_delete_policy, 'trash')
})

test('mail and calendar still refuse what they cannot do', () => {
  stubHttp([])
  assert.throws(
    () => opCreate(CREDENTIAL, mailMount(), { payload: { subject: 'x' } }),
    (e) => e.code === 'config_error' && /submit/.test(e.message)
  )
  stubHttp([])
  assert.throws(
    () => opDelete(CREDENTIAL, mailMount(), { item_id: 'MSG-1' }),
    (e) => e.code === 'config_error'
  )
  stubHttp([])
  assert.throws(
    () =>
      opDelete(CREDENTIAL, { sync_config: { resource: 'calendar' } }, {
        item_id: 'EV-1',
        policy: 'purge',
      }),
    (e) => e.code === 'config_error' && /no permanent delete/.test(e.message)
  )
})

// ---- drive delta paths ----------------------------------------------------
//
// A PRODUCTION INCIDENT. The backfill placed `Sales Deck.docx` inside `General`;
// two files added later and delivered by webhook landed FLAT at the mount root,
// alongside a stray `root` folder node. `opGetChanges` was answering
// `relative_path: external_id` — one segment — for every drive item, and the
// engine joins that to `mount_path` verbatim.
//
// The invariant is not "the delta produces a nice path". It is that the delta
// and the FULL WALK produce the SAME path for the same item, because the two
// disagreeing means a file sits in one place after a backfill and another after
// a webhook, forever — the engine keeps whichever it saw first.

/** A drive mount rooted at the drive root: no remote_root to resolve. */
function driveRootMount(extra) {
  return { sync_config: { resource: 'files' }, ...extra }
}

/**
 * What the ENGINE's full walk would build for these items.
 *
 * Mirrors `full.rs` `resolve_item_path` — `{prefix}/{item.name}`, where the
 * prefix is the parent folder's own resolved path — and the folder recursion
 * that feeds it. Written out rather than hand-computed so the equality assertion
 * below is against the real rule and not against a literal someone typed twice.
 */
function walkPaths(mount, tree) {
  const out = {}
  const visit = (folderId, prefix) => {
    const calls = stubHttp([{ body: { value: tree[folderId] || [] } }])
    void calls
    for (const item of opList(CREDENTIAL, mount, { folder_id: folderId }).items) {
      const rel = prefix ? `${prefix}/${item.name}` : item.name
      out[item.external_id] = rel
      if (item.is_folder) visit(item.external_id, rel)
    }
  }
  visit(null, '')
  return out
}

test('a delta file in a nested folder resolves under that folder, not at the root', () => {
  const token = mintCursor(driveRootMount())
  stubHttp([
    {
      body: {
        value: [
          {
            id: 'DOC-1',
            name: 'Sales Deck.docx',
            file: {},
            parentReference: { id: 'F-GENERAL', path: '/drive/root:/General' },
          },
        ],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const out = opGetChanges(CREDENTIAL, driveRootMount(), { since_token: token })
  assert.equal(out.items[0].relative_path, 'General/Sales Deck.docx')
})

test('a folder name with a space is DECODED, not materialized as %20', () => {
  // Graph percent-encodes every path segment. Left encoded, the file lands in a
  // folder called "Maravilla%20Accelerator" that the walk will never produce.
  const token = mintCursor(driveRootMount())
  stubHttp([
    {
      body: {
        value: [
          {
            id: 'IMG-1',
            name: 'maravilla-logo.png',
            file: {},
            parentReference: { path: '/drives/b!abc/root:/Maravilla%20Accelerator' },
          },
        ],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const out = opGetChanges(CREDENTIAL, driveRootMount(), { since_token: token })
  assert.equal(out.items[0].relative_path, 'Maravilla Accelerator/maravilla-logo.png')
})

test('the delta and the full walk resolve the SAME item to the SAME path', () => {
  // THE actual invariant. Asserting two literals would pass happily while both
  // paths were wrong in the same way; this compares the two code paths.
  const mount = driveRootMount()
  const folder = { id: 'F-GENERAL', name: 'General', folder: { childCount: 1 } }
  const doc = {
    id: 'DOC-1',
    name: 'Sales Deck.docx',
    file: {},
    parentReference: { id: 'F-GENERAL', path: '/drive/root:/General' },
  }
  const nested = {
    id: 'DOC-2',
    name: 'Q3.xlsx',
    file: {},
    parentReference: { id: 'F-SUB', path: '/drive/root:/General/Sub Folder' },
  }
  const sub = { id: 'F-SUB', name: 'Sub Folder', folder: { childCount: 1 } }

  const fromWalk = walkPaths(mount, {
    null: [folder],
    'F-GENERAL': [doc, sub],
    'F-SUB': [nested],
  })

  const token = mintCursor(mount)
  stubHttp([
    { body: { value: [doc, nested], '@odata.deltaLink': 'https://graph/d' } },
  ])
  const out = opGetChanges(CREDENTIAL, mount, { since_token: token })
  const fromDelta = Object.fromEntries(
    out.items.map((c) => [c.item.external_id, c.relative_path])
  )

  assert.equal(fromDelta['DOC-1'], fromWalk['DOC-1'])
  assert.equal(fromDelta['DOC-2'], fromWalk['DOC-2'])
  // And the layout is the human one, at both ends.
  assert.equal(fromWalk['DOC-2'], 'General/Sub Folder/Q3.xlsx')
})

test('a remote_root subfolder is stripped, so paths are relative to the MOUNT', () => {
  // The walk starts INSIDE remote_root with an empty prefix. Without stripping,
  // every delta path would carry the mount folder and disagree by a segment.
  const token = mintCursor(filesMount())
  stubHttp([
    {
      body: {
        value: [
          {
            id: 'DOC-1',
            name: 'a.txt',
            file: {},
            parentReference: { path: '/drive/root:/Team/Mounted/Inner' },
          },
          // Outside the mount root entirely: skipped rather than placed, because
          // the engine joins relative_path to mount_path verbatim.
          {
            id: 'DOC-2',
            name: 'elsewhere.txt',
            file: {},
            parentReference: { path: '/drive/root:/Other' },
          },
        ],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
    // The mount root resolution: ROOT is /Team/Mounted.
    { body: { id: 'ROOT', name: 'Mounted', parentReference: { path: '/drive/root:/Team' } } },
  ])
  const out = opGetChanges(CREDENTIAL, filesMount(), { since_token: token })
  assert.equal(out.items.length, 1, 'the item outside the mount root is skipped')
  assert.equal(out.items[0].relative_path, 'Inner/a.txt')
})

test('the mount root container itself is skipped, not materialized as a folder', () => {
  // The stray `root` node in the incident. Graph reports the container a delta
  // is scoped to as an item of that delta — a folder standing for the mount,
  // inside itself.
  const token = mintCursor(driveRootMount())
  stubHttp([
    {
      body: {
        value: [
          { id: 'DRIVE-ROOT', name: 'root', folder: { childCount: 2 }, root: {} },
          { id: 'DOC-1', name: 'a.txt', file: {}, parentReference: { path: '/drive/root:' } },
        ],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const out = opGetChanges(CREDENTIAL, driveRootMount(), { since_token: token })
  assert.deepEqual(out.items.map((c) => c.item.external_id), ['DOC-1'])
  assert.equal(out.items[0].relative_path, 'a.txt')

  // The same, for a mount rooted at a subfolder: that folder arrives as an item
  // of its own delta and carries no `root` facet.
  const t2 = mintCursor(filesMount())
  stubHttp([
    {
      body: {
        value: [{ id: 'ROOT', name: 'Mounted', folder: { childCount: 0 } }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  assert.deepEqual(opGetChanges(CREDENTIAL, filesMount(), { since_token: t2 }).items, [])
})

test('a deletion still carries the id as its path, and costs no root lookup', () => {
  // The engine's delete arm matches on external_id and never reads the path —
  // and a deleted entry carries no parentReference to derive one from.
  const token = mintCursor(filesMount())
  const calls = stubHttp([
    {
      body: {
        value: [{ id: 'GONE', name: 'old.txt', file: {}, deleted: { state: 'deleted' } }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const out = opGetChanges(CREDENTIAL, filesMount(), { since_token: token })
  assert.equal(out.items[0].type, 'deleted')
  assert.equal(out.items[0].relative_path, 'GONE')
  assert.equal(calls.length, 1, 'a page of nothing but deletions resolves no mount root')
})

test('an item with no parent path falls back to its name rather than inventing one', () => {
  const token = mintCursor(driveRootMount())
  stubHttp([
    {
      body: {
        value: [{ id: 'ODD-1', name: 'loose.txt', file: {} }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const out = opGetChanges(CREDENTIAL, driveRootMount(), { since_token: token })
  assert.equal(out.items[0].relative_path, 'loose.txt')
})

test('mail and calendar relative paths are untouched', () => {
  // A mail mount is ONE folder: the engine builds no prefix for it, the id is
  // the correct path, and a path_template is how an operator reshapes it.
  const token = mintCursor(mailMount())
  stubHttp([
    {
      body: {
        value: [{ id: 'MSG-9', subject: 'x', receivedDateTime: '2026-08-12T10:00:00Z' }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const mail = opGetChanges(CREDENTIAL, mailMount(), { since_token: token })
  assert.equal(mail.items[0].relative_path, 'MSG-9')
  assert.equal(mail.items[0].item.name, 'MSG-9', 'and a mail item is still NAMED by its id')

  const calMount = { sync_config: { resource: 'calendar' } }
  const calToken = mintCursor(calMount)
  stubHttp([
    {
      body: {
        value: [{ id: 'EV-9', subject: 'Standup', type: 'singleInstance' }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const cal = opGetChanges(CREDENTIAL, calMount, { since_token: calToken })
  assert.equal(cal.items[0].relative_path, 'EV-9')
})

// ---- attachments: the derived-type select ---------------------------------
//
// Production incident: `$select=…,contentId` on `/attachments` returns
//
//   Could not find a property named 'contentId' on type 'microsoft.graph.attachment'
//
// because contentId belongs to the DERIVED fileAttachment. Graph rejects the
// whole request with a 400, the adapter classifies it `config_error`, and the
// mount stops after three failures — for every message, not only ones with
// attachments. A mailbox that had just enabled attachments stopped syncing.

test('the attachment select casts to the derived type that owns contentId', () => {
  const calls = stubHttp([
    { body: { value: [{ id: 'M1', subject: 's' }] } },
    { body: { value: [] } },
  ])
  opList(CREDENTIAL, mailMount({ sync_config: { resource: 'mail', include_attachments: true } }), {})

  const listing = calls.find((c) => /\/attachments\?/.test(c.url))
  assert.ok(listing, 'attachments must be listed when the mount opted in')
  assert.match(
    listing.url,
    /microsoft\.graph\.fileAttachment\/contentId/,
    'contentId is not on the base attachment type; unqualified it 400s the ' +
      'whole request and stops the mount',
  )
})

test('a select Graph will not accept degrades to no contentId, not to a dead mount', () => {
  const calls = stubHttp([
    { body: { value: [{ id: 'M1', subject: 's' }] } },
    {
      status: 400,
      body: {
        error: {
          code: 'RequestBroker--ParseUri',
          message:
            "Parsing OData Select and Expand failed: Could not find a property " +
            "named 'contentId' on type 'microsoft.graph.attachment'.",
        },
      },
    },
    { body: { value: [{ id: 'A1', name: 'logo.png', isInline: true }] } },
  ])

  const out = opList(
    CREDENTIAL,
    mailMount({ sync_config: { resource: 'mail', include_attachments: true } }),
    {},
  )

  const retried = calls.filter((c) => /\/attachments\?/.test(c.url))
  assert.equal(retried.length, 2, 'the rejected select must be retried once')
  assert.doesNotMatch(retried[1].url, /contentId/, 'the retry drops the cast')

  const item = out.items[0]
  assert.equal(item.metadata.attachments.length, 1, 'the attachment still imports')
  assert.equal(
    item.metadata.attachments[0].content_id,
    null,
    'only the cid: reference is lost, and it is lost honestly',
  )
  assert.ok(!item.metadata.children_unknown, 'this is a known listing, not an unknown one')
})

test('a 400 about anything else still fails the mount', () => {
  stubHttp([
    { body: { value: [{ id: 'M1', subject: 's' }] } },
    { status: 400, body: { error: { code: 'ErrorInvalidUser', message: 'The mailbox is unavailable.' } } },
  ])
  assert.throws(
    () =>
      opList(
        CREDENTIAL,
        mailMount({ sync_config: { resource: 'mail', include_attachments: true } }),
        {},
      ),
    /mailbox is unavailable/i,
    'a select fallback must not swallow unrelated 400s',
  )
})
