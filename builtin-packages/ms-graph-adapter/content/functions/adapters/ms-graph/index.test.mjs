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
import { MAIL_BODY_PAGE, MAIL_PAGE, outlookHeaders, useImmutableIds } from './mount.js'
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
  // A locally-created folder becomes a real one at the provider (driveCreate's
  // folder branch). Still no command surface: a drive item is not a command.
  assert.equal(files.can_create_folders, true)
  assert.equal(files.can_submit, undefined)

  // Mail keeps update + submit and nothing else; the drive work must not have
  // widened it.
  const mail = opCapabilities(mailMount())
  assert.equal(mail.can_update, true)
  assert.equal(mail.can_submit, true)
  assert.equal(mail.can_create, undefined)
  assert.equal(mail.can_delete, undefined)
  assert.equal(mail.accepts_content, undefined)
  // Two spellings of the read flag, plus importance. The follow-up FLAG is
  // imported but not writable, so it must NOT appear here — declaring a field
  // the mapper cannot translate is how a push resolves and then throws.
  assert.deepEqual(mail.mutable_fields, ['unread', 'is_read', 'importance'])

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

// ---- the follow-up flag, and importance -----------------------------------

test('a follow-up flag arrives as the provider-neutral "flagged"', () => {
  stubHttp([
    {
      body: {
        value: [
          { id: 'M1', subject: 'flagged one', flag: { flagStatus: 'flagged' } },
          { id: 'M2', subject: 'finished', flag: { flagStatus: 'complete' } },
          { id: 'M3', subject: 'plain', flag: { flagStatus: 'notFlagged' } },
          { id: 'M4', subject: 'no flag key at all' },
        ],
      },
    },
  ])
  const out = opList(CREDENTIAL, mailMount(), {})
  const flags = out.items.map((i) => i.metadata.flags)

  assert.deepEqual(flags[0], ['flagged'])
  // `complete` is a FINISHED follow-up and `notFlagged` is the absence of one;
  // both would read as flagged to anyone testing the array for members.
  assert.equal(flags[1], null)
  assert.equal(flags[2], null)
  assert.equal(flags[3], null)
})

test('the mail select asks for the flag', () => {
  const calls = stubHttp([{ body: { value: [] } }])
  opList(CREDENTIAL, mailMount(), {})
  assert.match(
    decodeURIComponent(calls[0].url),
    /\bflag\b/,
    'without it in $select Graph never returns the flag and every message reads as unflagged',
  )
})

// ---- a create files into the node's OWN folder -----------------------------
//
// Production incident: every uploaded file landed at the top of the SharePoint
// library whatever folder it was uploaded into, because the engine told the
// adapter only the mount's remote root. The walk then re-placed the local node
// at the root to match, so the wrong destination propagated back and read as
// "the sync moved my file".

test('a create files into the parent folder the engine resolved', () => {
  const calls = stubHttp([{ body: { id: '01NEW', '@odata.etag': '"1"' } }])
  opCreate(CREDENTIAL, filesMount(), {
    payload: { name: 'logo.svg' },
    parent_id: 'ROOT',
    parent_external_id: 'FOLDER-GRUENDUNG',
    content: { name: 'logo.svg', mime_type: 'image/svg+xml', size: 3, inline: true, content_base64: 'AQID' },
  })
  assert.match(
    calls[0].url,
    /\/items\/FOLDER-GRUENDUNG:/,
    'the node\'s own folder wins over the mount root',
  )
})

test('a create at the mount root still uses the remote root', () => {
  const calls = stubHttp([{ body: { id: '01NEW', '@odata.etag': '"1"' } }])
  opCreate(CREDENTIAL, filesMount(), {
    payload: { name: 'logo.svg' },
    parent_id: 'ROOT',
    // No parent_external_id: the node sits directly under the mount path, or
    // its parent folder does not exist at the provider yet.
    content: { name: 'logo.svg', mime_type: 'image/svg+xml', size: 3, inline: true, content_base64: 'AQID' },
  })
  assert.match(calls[0].url, /\/items\/ROOT:/, 'falls back to the mount root')
})

// ---- a deletion carries a name, and a page survives it ---------------------
//
// Production: deleting a file in RaisinDB mirrored the delete to OneDrive
// correctly, then Graph reported that deletion on the delta feed — and the
// engine answered "bad get_changes response: missing field `name`". One
// deletion failed the WHOLE page to deserialize, so 36 items were processed and
// none applied, on every poll.

test('a deleted change carries a name so the page still parses', () => {
  stubHttp([
    {
      body: {
        value: [
          { id: 'D1', deleted: { state: 'deleted' } },
          // Inside the mount root, so it is in scope for this mount.
          {
            id: 'K1',
            name: 'kept.txt',
            file: {},
            parentReference: { path: '/drive/root:/root-folder' },
          },
        ],
        '@odata.deltaLink': 'https://graph.microsoft.com/next',
      },
    },
    // The lazy mount-root resolve: `filesMount()` is rooted at a subfolder, so
    // the adapter asks once per page where that folder sits.
    { body: { id: 'ROOT', name: 'root-folder', parentReference: { path: '/drive/root:' } } },
  ])
  const out = opGetChanges(CREDENTIAL, filesMount(), { since_token: null })

  const removed = out.items.find((c) => c.type === 'deleted')
  assert.ok(removed, 'the deletion must be reported')
  assert.equal(removed.item.external_id, 'D1')
  assert.ok(removed.item.name, 'name is required by the engine item shape')
  // And the rest of the page still arrives — the point of the fix.
  assert.ok(out.items.some((c) => c.type === 'updated' && c.item.external_id === 'K1'))
})

// ---- folder creation -------------------------------------------------------

test('a folder create POSTs a folder facet into its parent', () => {
  const calls = stubHttp([{ body: { id: '01FOLDER', '@odata.etag': '"1"' } }])
  const out = opCreate(CREDENTIAL, filesMount(), {
    payload: { name: 'Reports', is_folder: true },
    parent_id: 'ROOT',
    parent_external_id: 'FOLDER-PARENT',
  })

  assert.match(calls[0].url, /\/items\/FOLDER-PARENT\/children$/, 'created inside its parent')
  assert.equal(calls[0].request.method, 'POST')
  assert.deepEqual(calls[0].request.body.folder, {})
  assert.equal(calls[0].request.body.name, 'Reports')
  // `rename`, not `replace`: the folder already at that name may be someone
  // else's, and taking it over would silently merge two trees.
  assert.equal(calls[0].request.body['@microsoft.graph.conflictBehavior'], 'rename')
  assert.equal(out.external_id, '01FOLDER')
})

test('a create with no content and no is_folder is still treated as a folder', () => {
  // The engine defers a create whose bytes have not arrived, so "no content"
  // here means the node has none — not that they are still coming.
  const calls = stubHttp([{ body: { id: '01F2' } }])
  opCreate(CREDENTIAL, filesMount(), { payload: { name: 'Loose' }, parent_id: 'ROOT' })
  assert.match(calls[0].url, /\/children$/)
})

// ---- mail folder TREE mounts ----------------------------------------------
//
// Graph's message delta is FOLDER-SCOPED — v1.0 documents only
// `/mailFolders/{id}/messages/delta`, there is no mailbox-wide feed — so a
// mount that spans a subtree holds ONE DELTA LINK PER FOLDER inside the single
// opaque cursor the engine stores. Everything below is a failure that shape
// makes available:
//
//  * the walk and the delta disagreeing about a message's path by one segment,
//    which relocates the node on every run (it shipped twice already: once on
//    google-drive, once on ms-graph drive)
//  * a NEW folder seeded with `$deltatoken=latest`, silently dropping an entire
//    folder's history with nothing to observe
//  * a folder RENAME that changes no message etag, so `can_skip_unmapped`
//    returns before rel_path is read and every message stays at its old path
//    forever — repairable only by force_rewrite
//  * an `@removed` emitted as a delete, which Graph also sends for a MOVE OUT
//    of the folder — so filing an email destroys its node
//  * an unbounded or unpersisted folder rotation, which starves every quiet
//    folder behind one busy one and still reports `ok`

import { MAIL_TREE_SLICE, entryUrl } from './changes.js'
import { folderSegment } from './mail-folders.js'
import { subscriptionResource } from './subscribe.js'

/** A URL-routed host stub: a walk makes many calls and their order is the
 *  engine's business, not the test's. */
function stubRouter(route) {
  const calls = []
  globalThis.raisin = {
    http: {
      fetch(url, request) {
        calls.push({ url, request })
        const r = route(url)
        if (!r) throw new Error(`unrouted request: ${url}`)
        return { status: 200, headers: {}, body: {}, ...r }
      },
    },
  }
  return calls
}

/** A deltaLink shaped the way Graph really mints one: the whole query frozen
 *  into it, including the ~330-char URL-encoded $select the cursor must NOT
 *  store a hundred copies of. */
function deltaLinkFor(id) {
  return (
    `https://graph.microsoft.com/v1.0/me/mailFolders/${id}/messages/delta` +
    `?%24select=${encodeURIComponent('id,subject,from,toRecipients,receivedDateTime,parentFolderId')}` +
    `&%24deltatoken=TOK-${id}`
  )
}

function treeMount(extra) {
  return {
    remote_root: 'inbox',
    sync_config: { resource: 'mail', folder_scope: 'tree' },
    ...extra,
  }
}

/** Build a router over a fixture mailbox. */
function mailboxRouter(fx) {
  const folderById = { [fx.root.id]: fx.root }
  for (const f of fx.folders) folderById[f.id] = f
  const idOf = (url) => {
    const m = /\/mailFolders\/([^/?]+)/.exec(url)
    return m ? decodeURIComponent(m[1]) : null
  }
  const resolve = (id) => (id === 'inbox' ? fx.root : folderById[id])
  const deltaBody = (id, url) => {
    if (fx.delta) {
      const custom = fx.delta(id, url)
      if (custom) return custom
    }
    return {
      body: {
        value: (fx.messages[id] || []).slice(),
        '@odata.deltaLink': deltaLinkFor(id),
      },
    }
  }
  return (url) => {
    const id = idOf(url)
    if (!id) return null
    if (url.includes('/childFolders')) {
      const parent = resolve(id).id
      return { body: { value: fx.folders.filter((f) => f.parentFolderId === parent) } }
    }
    if (url.includes('/messages/delta')) return deltaBody(resolve(id).id, url)
    if (url.includes('/messages')) {
      return { body: { value: (fx.messages[resolve(id).id] || []).slice() } }
    }
    return { body: resolve(id) || null }
  }
}

/** Drive `opList` the way `full.rs` drives it: an explicit folder stack, the
 *  prefix accumulated from each folder item's own resolved path. */
function mailWalkPaths(mount, route) {
  const out = {}
  const stack = [[mount.remote_root || null, '']]
  while (stack.length) {
    const [folderId, prefix] = stack.pop()
    let cursor = null
    do {
      stubRouter(route)
      const page = opList(CREDENTIAL, mount, { folder_id: folderId, cursor, limit: 500 })
      for (const item of page.items) {
        const rel = prefix ? `${prefix}/${item.name}` : item.name
        out[item.external_id] = rel
        if (item.is_folder) stack.push([item.external_id, rel])
      }
      cursor = page.next_cursor
    } while (cursor)
  }
  return out
}

const FIXTURE = {
  root: { id: 'F-INBOX', displayName: 'Inbox', parentFolderId: null, childFolderCount: 2 },
  folders: [
    { id: 'F-PROJ', displayName: 'Projects', parentFolderId: 'F-INBOX', childFolderCount: 1 },
    // A slash in an Outlook folder name is ONE folder, not two path segments.
    { id: 'F-ACME', displayName: 'Acme/Corp', parentFolderId: 'F-PROJ', childFolderCount: 0 },
    { id: 'F-NEWS', displayName: 'Newsletters', parentFolderId: 'F-INBOX', childFolderCount: 0 },
  ],
  messages: {
    'F-INBOX': [msg('M-ROOT', 'F-INBOX')],
    'F-PROJ': [msg('M-PROJ', 'F-PROJ')],
    'F-ACME': [msg('M-ACME', 'F-ACME')],
    'F-NEWS': [msg('M-NEWS', 'F-NEWS')],
  },
}

function msg(id, parentFolderId, etag) {
  return {
    id,
    parentFolderId,
    subject: `subject ${id}`,
    receivedDateTime: '2026-08-12T10:00:00Z',
    '@odata.etag': etag || `W/"${id}-1"`,
  }
}

test('the mail tree walk and the mail tree delta resolve every message to the SAME path', () => {
  // THE invariant, written as a property over the whole fixture rather than as
  // two literals — two literals would pass happily while both paths were wrong
  // in the same way. `get_changes`' relative_path must equal the walk's
  // {prefix}/{item.name} for the SAME message, or the engine relocates the node
  // on every run where they disagree.
  const mount = treeMount()
  const route = mailboxRouter(FIXTURE)

  const fromWalk = mailWalkPaths(mount, route)

  stubRouter(route)
  const out = opGetChanges(CREDENTIAL, mount, {})
  const fromDelta = Object.fromEntries(
    out.items.map((c) => [c.item.external_id, c.relative_path])
  )

  for (const id of ['M-ROOT', 'M-PROJ', 'M-ACME', 'M-NEWS']) {
    assert.equal(
      fromDelta[id],
      fromWalk[id],
      `${id}: the delta path and the walk path must be byte-identical`
    )
  }
  // And the layout is the concrete one, at both ends — so a shared bug in both
  // resolvers cannot make the property above vacuously true.
  assert.equal(fromWalk['M-ROOT'], 'M-ROOT')
  assert.equal(fromWalk['M-PROJ'], 'Projects/M-PROJ')
  assert.equal(fromWalk['M-ACME'], 'Projects/Acme-Corp/M-ACME')
  assert.equal(fromWalk['M-NEWS'], 'Newsletters/M-NEWS')
  // The folder NODES land on the chain their children hang from.
  assert.equal(fromWalk['F-ACME'], 'Projects/Acme-Corp')
})

test('a folder-mode mail mount is untouched by any of this', () => {
  // Every existing mount keeps today's behaviour byte for byte: one flat
  // listing of one folder, the id as the path, no folder requests at all.
  const calls = stubHttp([{ body: { value: [msg('MSG-1', 'F-INBOX')] } }])
  const page = opList(CREDENTIAL, mailMount(), {})
  assert.equal(calls.length, 1, 'folder mode resolves no folder map')
  assert.match(calls[0].url, /\/mailFolders\/inbox\/messages\?/)
  assert.doesNotMatch(calls[0].url, /parentFolderId/, 'the projection is unwidened, so no stored cursor is invalidated')
  assert.equal(page.items.length, 1)
  assert.equal(page.items[0].etag, 'W/"MSG-1-1"', 'and the etag is the bare provider one')
})

test('a new folder is seeded with an ENUMERATION, never with $deltatoken=latest', () => {
  // The mount-level rule is INVERTED for a folder that appeared later: none of
  // its messages has ever been imported, so `latest` would drop the whole
  // folder's history with nothing anywhere to observe.
  const mount = treeMount()
  stubRouter(mailboxRouter(FIXTURE))
  const first = opGetChanges(CREDENTIAL, mount, {})

  const grown = {
    ...FIXTURE,
    root: { ...FIXTURE.root, childFolderCount: 3 },
    folders: [
      ...FIXTURE.folders,
      { id: 'F-NEW', displayName: 'Invoices', parentFolderId: 'F-INBOX', childFolderCount: 0 },
    ],
    messages: { ...FIXTURE.messages, 'F-NEW': [msg('M-OLD', 'F-NEW')] },
  }
  const calls = stubRouter(mailboxRouter(grown))
  const second = opGetChanges(CREDENTIAL, mount, { since_token: first.next_token })

  const seeded = calls.filter((c) => c.url.includes('F-NEW'))
  assert.ok(seeded.length, 'the new folder is polled')
  for (const c of seeded) {
    assert.doesNotMatch(
      c.url,
      /deltatoken=latest/,
      'a new folder seeded with `latest` silently loses everything already in it'
    )
  }
  const paths = Object.fromEntries(second.items.map((c) => [c.item.external_id, c.relative_path]))
  assert.equal(paths['M-OLD'], 'Invoices/M-OLD', 'and its existing mail arrives at its real path')
})

test('a baseline call fetches nothing and seeds every folder from now on', () => {
  // `capture_delta_baseline` discards the items and keeps only the token, so
  // pulling pages there is pure cost — and after a completed walk "from now on"
  // is the truth for every folder at once.
  const calls = stubRouter(mailboxRouter(FIXTURE))
  const out = opGetChanges(CREDENTIAL, treeMount(), { since_token: null, baseline_only: true })
  assert.deepEqual(out.items, [])
  assert.equal(out.has_more, false)
  assert.equal(
    calls.filter((c) => c.url.includes('/messages/delta')).length,
    0,
    'a baseline reads no message feed at all'
  )
  const stored = readTree(out.next_token)
  for (const fid of Object.keys(stored.m)) {
    assert.equal(stored.m[fid].t, 'latest')
    assert.equal(stored.m[fid].s, 'delta')
    // And the token still rebuilds the exact URL the old cursor stored whole.
    assert.match(entryUrl(treeMount(), fid, stored.m[fid]), /deltatoken=latest/)
  }
})

test('a folder RENAME re-emits its messages at the new path, with a new etag', () => {
  // `batch.rs` can_skip_unmapped compares external_id + etag and RETURNS BEFORE
  // rel_path is consulted. An Outlook rename changes no message etag, so
  // without the folder path folded into the etag every message in the renamed
  // folder is skipped as unchanged and stays at its OLD path forever — a full
  // walk does not repair it either, only force_rewrite does.
  const mount = treeMount()
  stubRouter(mailboxRouter(FIXTURE))
  const first = opGetChanges(CREDENTIAL, mount, {})
  const before = first.items.find((c) => c.item.external_id === 'M-PROJ')
  assert.equal(before.relative_path, 'Projects/M-PROJ')

  const renamed = {
    ...FIXTURE,
    folders: FIXTURE.folders.map((f) =>
      f.id === 'F-PROJ' ? { ...f, displayName: 'Programs' } : f
    ),
  }
  const calls = stubRouter(mailboxRouter(renamed))
  const second = opGetChanges(CREDENTIAL, mount, { since_token: first.next_token })

  const projCalls = calls.filter((c) => c.url.includes('F-PROJ') && c.url.includes('delta'))
  assert.ok(
    projCalls.some((c) => c.url.includes('/messages/delta?$select=')),
    'the renamed folder is re-ENUMERATED; its stored delta link would report ' +
      'nothing at all, because none of its messages changed'
  )
  const after = second.items.find((c) => c.item.external_id === 'M-PROJ')
  assert.equal(after.relative_path, 'Programs/M-PROJ')
  assert.notEqual(
    after.item.etag,
    before.item.etag,
    'the etag must move with the path, or the engine skips the relocation'
  )
  assert.match(after.item.etag, /\|p=Programs$/)
  // The message underneath is untouched: a rename is not a re-import.
  assert.equal(after.item.external_id, before.item.external_id)
  // Its CHILD folder moves with it, and so does the grandchild's mail.
  assert.equal(
    second.items.find((c) => c.item.external_id === 'M-ACME').relative_path,
    'Programs/Acme-Corp/M-ACME'
  )
})

test('an @removed in tree mode emits NO delete, because Graph cannot tell one from a move', () => {
  // Microsoft documents @removed reason:"deleted" as covering an item DELETED
  // OR MOVED FROM the folder, as a collection-level event. The destination
  // folder's create carries the same immutable id on a different feed with no
  // ordering guarantee, so emitting the delete races the create and destroys
  // the node under the ordinary act of filing an email.
  const mount = treeMount()
  stubRouter(mailboxRouter(FIXTURE))
  const first = opGetChanges(CREDENTIAL, mount, {})

  const moved = {
    ...FIXTURE,
    delta: (id) => {
      if (id === 'F-INBOX') {
        return {
          body: {
            value: [{ id: 'M-ROOT', '@removed': { reason: 'deleted' } }],
            '@odata.deltaLink': deltaLinkFor('F-INBOX'),
          },
        }
      }
      if (id === 'F-NEWS') {
        return {
          body: {
            value: [msg('M-ROOT', 'F-NEWS')],
            '@odata.deltaLink': deltaLinkFor('F-NEWS'),
          },
        }
      }
      return null
    },
  }
  stubRouter(mailboxRouter(moved))
  const second = opGetChanges(CREDENTIAL, mount, { since_token: first.next_token })

  assert.equal(
    second.items.filter((c) => c.type === 'deleted').length,
    0,
    'the walk reconcile is the only remover in tree mode'
  )
  const relocated = second.items.find((c) => c.item.external_id === 'M-ROOT')
  assert.equal(relocated.type, 'updated')
  assert.equal(relocated.relative_path, 'Newsletters/M-ROOT', 'the move lands as a relocation')
})

test('folder mode still deletes on @removed', () => {
  // The ambiguity is real there too, but a one-folder mount has nowhere else
  // inside itself for the message to go — so the existing arm is unchanged.
  const token = mintCursor(mailMount())
  stubHttp([
    {
      body: {
        value: [{ id: 'MSG-GONE', '@removed': { reason: 'deleted' } }],
        '@odata.deltaLink': 'https://graph/d',
      },
    },
  ])
  const out = opGetChanges(CREDENTIAL, mailMount(), { since_token: token })
  assert.equal(out.items[0].type, 'deleted')
  assert.equal(out.items[0].relative_path, 'MSG-GONE')
})

/** The tree cursor, unwrapped. */
function readTree(token) {
  return JSON.parse(token.slice(token.indexOf(':') + 1))
}

/** A mailbox that is `count` flat folders under the root. */
function wideMailbox(count) {
  const fx = {
    root: { id: 'F-INBOX', displayName: 'Inbox', parentFolderId: null, childFolderCount: count },
    folders: [],
    messages: {},
  }
  for (let i = 1; i <= count; i++) {
    const id = `F-${i}`
    fx.folders.push({ id, displayName: `Folder ${i}`, parentFolderId: 'F-INBOX', childFolderCount: 0 })
    fx.messages[id] = [msg(`M-${i}`, id)]
  }
  return fx
}

const polledFolders = (calls) =>
  new Set(
    calls
      .filter((c) => c.url.includes('/messages/delta'))
      .map((c) => /mailFolders\/(F-[A-Z0-9]+)/.exec(c.url)[1])
  )

test('the folder rotation advances, wraps, and starves nobody', () => {
  // Unpersisted or unbounded, one busy folder consumes the whole
  // max_items_per_sync budget every run and the other N-1 never advance — with
  // items written and nothing anywhere saying the rest of the mailbox is
  // standing still.
  const wide = wideMailbox(6)
  const n = 1 + wide.folders.length // the root counts as a folder of the tree
  assert.ok(n > MAIL_TREE_SLICE, 'the fixture must be wider than one slice')

  const mount = treeMount()

  let calls = stubRouter(mailboxRouter(wide))
  const first = opGetChanges(CREDENTIAL, mount, {})
  assert.equal(first.has_more, true, 'folders are still unvisited this round')
  const firstPolled = polledFolders(calls)
  assert.equal(firstPolled.size, MAIL_TREE_SLICE, 'exactly one slice per call')
  // THE RESUME POINT IS A FOLDER ID, not a position — see the next test.
  assert.deepEqual([...firstPolled].sort(), ['F-1', 'F-2', 'F-3', 'F-4', 'F-5'])
  assert.equal(readTree(first.next_token).r, 'F-6', 'the resume point names the folder to visit next')

  calls = stubRouter(mailboxRouter(wide))
  const second = opGetChanges(CREDENTIAL, mount, { since_token: first.next_token })
  assert.equal(readTree(second.next_token).r, null, 'the round closes once every folder has been visited')
  assert.equal(second.has_more, false, 'and only then is the mount caught up')
  const secondPolled = polledFolders(calls)
  for (const f of firstPolled) {
    assert.ok(!secondPolled.has(f), `${f} was polled twice while others waited`)
  }
  assert.equal(firstPolled.size + secondPolled.size, n, 'every folder is reached in one round')
})

test('the rotation resumes at a folder ID, so adding a folder shifts nothing', () => {
  // `order` is SORTED, so a persisted INDEX means one folder created or deleted
  // since the last call moves every folder after it: the resumed slice then
  // skipped a folder or visited one twice, silently, for as long as the mailbox
  // kept changing. Only an id survives a re-sort.
  const wide = wideMailbox(6)
  const mount = treeMount()

  stubRouter(mailboxRouter(wide))
  const first = opGetChanges(CREDENTIAL, mount, {})
  const cur = readTree(first.next_token)
  const resumeAt = cur.r
  assert.equal(typeof resumeAt, 'string', 'the resume point is an id, not a number')

  // A folder appears that sorts BEFORE the resume point — the exact shift that
  // moved an index off by one. Injected into the cursor because that is what a
  // mid-round rebuild-on-miss leaves behind.
  cur.m['F-0'] = { t: null, s: 'enum', p: 'Folder 0' }
  const shifted = 'rsn-mailtree-2:' + JSON.stringify(cur)

  const grown = {
    ...wide,
    root: { ...wide.root, childFolderCount: 7 },
    folders: [
      { id: 'F-0', displayName: 'Folder 0', parentFolderId: 'F-INBOX', childFolderCount: 0 },
      ...wide.folders,
    ],
    messages: { ...wide.messages, 'F-0': [msg('M-0', 'F-0')] },
  }
  const calls = stubRouter(mailboxRouter(grown))
  opGetChanges(CREDENTIAL, mount, { since_token: shifted })
  const polled = polledFolders(calls)

  assert.ok(polled.has(resumeAt), `the rotation resumed at ${resumeAt}, the folder it named`)
  for (const f of ['F-1', 'F-2', 'F-3', 'F-4', 'F-5']) {
    assert.ok(!polled.has(f), `${f} was already visited this round and must not repeat`)
  }
})

test('the rotation resumes past a folder that has been DELETED since', () => {
  // The persisted id can name a folder that is simply gone. Landing on the next
  // one in sort order is the only answer that neither restarts the round nor
  // skips the folders after it.
  const wide = wideMailbox(6)
  const mount = treeMount()
  stubRouter(mailboxRouter(wide))
  const cur = readTree(opGetChanges(CREDENTIAL, mount, {}).next_token)
  assert.equal(cur.r, 'F-6')
  delete cur.m['F-6']
  const token = 'rsn-mailtree-2:' + JSON.stringify(cur)

  const calls = stubRouter(mailboxRouter(wide))
  const out = opGetChanges(CREDENTIAL, mount, { since_token: token })
  assert.deepEqual([...polledFolders(calls)], ['F-INBOX'], 'it lands on the next folder, not the first')
  assert.equal(readTree(out.next_token).r, null, 'and the round still closes')
})

test('a hidden folder under a parent Graph calls CHILDLESS still reaches the delta', () => {
  // The walk and the delta have to discover the SAME folders. The map used to
  // skip listing a folder whose `childFolderCount` was 0, while the walk lists
  // unconditionally — and this adapter passes includeHiddenFolders precisely
  // BECAUSE hidden folders are missing from a default listing, so a count taken
  // from that view can say 0 over a real subtree. The walk would then materialize
  // the folder and its mail while the delta never polled it: those messages
  // arrived only after a complete full walk, if ever.
  const sneaky = {
    root: { ...FIXTURE.root, childFolderCount: 2 },
    folders: [
      ...FIXTURE.folders,
      // Graph reports Newsletters as a leaf, and it is not: Clutter hangs under
      // it and is hidden, which is exactly the pair that hid the subtree.
      { id: 'F-CLUTTER', displayName: 'Clutter', parentFolderId: 'F-NEWS', childFolderCount: 0, isHidden: true },
    ],
    messages: { ...FIXTURE.messages, 'F-CLUTTER': [msg('M-CLUTTER', 'F-CLUTTER')] },
  }
  const mount = treeMount()
  const route = mailboxRouter(sneaky)

  const fromWalk = mailWalkPaths(mount, route)
  assert.equal(fromWalk['M-CLUTTER'], 'Newsletters/Clutter/M-CLUTTER', 'the walk finds it either way')

  stubRouter(route)
  const out = opGetChanges(CREDENTIAL, mount, {})
  const fromDelta = Object.fromEntries(
    out.items.map((c) => [c.item.external_id, c.relative_path])
  )
  assert.equal(
    fromDelta['M-CLUTTER'],
    fromWalk['M-CLUTTER'],
    'the delta must poll the hidden folder and place its mail exactly where the walk does'
  )
})

test('the folder map is built ONCE PER ROUND, not once per poll', () => {
  // THE RATE LIMIT. `buildFolderMap` costs 1 + (folders with children) requests
  // and used to run on EVERY get_changes call, idle polls included. With a slice
  // of 5, a 100-folder tree paid for a whole map build 20 times per round; on a
  // folder-heavy mailbox that approaches ~2,000 childFolders requests per round
  // for ONE mount, against a Graph ceiling of 10,000 per 10 minutes per app per
  // mailbox. Proving the cost here rather than asserting it in a comment.
  const wide = wideMailbox(12)
  // Every folder has a child, so a map build is maximally expensive: one
  // childFolders request per folder, not the cheap all-leaves case.
  for (const f of [...wide.folders]) {
    f.childFolderCount = 1
    const kid = `${f.id}-K`
    wide.folders.push({ id: kid, displayName: `Kid of ${f.displayName}`, parentFolderId: f.id, childFolderCount: 0 })
    wide.messages[kid] = []
  }
  const n = 1 + wide.folders.length
  const rounds = Math.ceil(n / MAIL_TREE_SLICE)
  assert.ok(rounds >= 4, 'the fixture must span several polls per round')

  const mapCost = (calls) =>
    calls.filter((c) => c.url.includes('/childFolders') || /mailFolders\/[^/]+\?/.test(c.url)).length

  const mount = treeMount()
  let token = null
  const perPoll = []
  for (let i = 0; i < rounds; i++) {
    const calls = stubRouter(mailboxRouter(wide))
    const out = opGetChanges(CREDENTIAL, mount, token ? { since_token: token } : {})
    perPoll.push(mapCost(calls))
    token = out.next_token
  }

  assert.ok(perPoll[0] > 1, 'the first poll of a round does build the map')
  for (let i = 1; i < perPoll.length; i++) {
    assert.equal(
      perPoll[i],
      0,
      `poll ${i + 1} of the round rebuilt the folder map (${perPoll[i]} requests); ` +
        'the chains are cached in the cursor precisely so it does not'
    )
  }
  assert.equal(readTree(token).r, null, 'the round did close, so the next poll rebuilds')
  // And the round boundary is where the rebuild comes back — not never.
  const boundary = stubRouter(mailboxRouter(wide))
  opGetChanges(CREDENTIAL, mount, { since_token: token })
  assert.ok(mapCost(boundary) > 1, 'a closed round rebuilds, or a rename is never seen')
})

test('the cursor stores delta TOKENS, not whole links', () => {
  // `delta.rs` rewrites this blob after every page. A Graph message-delta link is
  // dominated by the URL-encoded $select — ~330 chars once include_body and
  // parentFolderId are on — so at max_folders 100 the old cursor spent ~33 KB of
  // ~95 KB storing one constant a hundred times. It is reconstructible from the
  // cursor IDENTITY, which already pins $select.
  const wide = wideMailbox(20)
  const mount = treeMount()
  stubRouter(mailboxRouter(wide))
  let out = opGetChanges(CREDENTIAL, mount, {})
  // Run the rotation to completion so every folder holds a real Graph token.
  for (let guard = 0; out.has_more && guard < 50; guard++) {
    stubRouter(mailboxRouter(wide))
    out = opGetChanges(CREDENTIAL, mount, { since_token: out.next_token })
  }
  assert.equal(out.has_more, false, 'the rotation must terminate')
  const stored = readTree(out.next_token)

  assert.equal(Object.keys(stored.m).length, 21)
  for (const [fid, e] of Object.entries(stored.m)) {
    assert.equal(e.t, `TOK-${fid}`, 'the token alone is kept')
    assert.equal(e.u, undefined, 'and never the whole link beside it')
    // The rebuilt URL is the query Graph minted, $select and all.
    const rebuilt = entryUrl(mount, fid, e)
    assert.match(rebuilt, /\$deltatoken=TOK-F-/)
    assert.match(rebuilt, /\$select=/)
  }
  assert.doesNotMatch(
    JSON.stringify(stored.m),
    /select|receivedDateTime/,
    'not one of the 21 entries may carry a copy of the projection'
  )
  // The identity keeps the ONE copy that makes the rebuild sound, and that is
  // the whole saving: one projection, not one per folder.
  assert.match(stored.k, /sel=id,subject/)

  // Measured against what the previous shape stored: the same cursor with each
  // entry holding its whole Graph link.
  const asLinks = JSON.stringify({
    k: stored.k,
    r: stored.r,
    i: stored.i,
    m: Object.fromEntries(
      Object.keys(stored.m).map((fid) => [fid, { u: deltaLinkFor(fid), p: stored.m[fid].p, s: 'delta' }])
    ),
  })
  const saved = 1 - out.next_token.length / asLinks.length
  assert.ok(saved > 0.5, `the cursor should shrink by more than half; it shrank by ${(saved * 100) | 0}%`)
  // Identity still guards it: the reconstructed $select is only correct because
  // a changed projection invalidates the cursor rather than replaying.
  assert.throws(
    () =>
      opGetChanges(
        CREDENTIAL,
        treeMount({ sync_config: { resource: 'mail', folder_scope: 'tree', include_body: true } }),
        { since_token: out.next_token }
      ),
    /different query/
  )
})

test('a link whose token cannot be read is kept whole, never rebuilt from a guess', () => {
  // Degrading to the old size is fine; rebuilding a query from a token we did
  // not actually find is how a cursor silently starts replaying the wrong feed.
  const opaque = {
    ...FIXTURE,
    delta: (id) => ({
      body: { value: [], '@odata.deltaLink': `https://graph.microsoft.com/opaque/${id}` },
    }),
  }
  const mount = treeMount()
  stubRouter(mailboxRouter(opaque))
  const out = opGetChanges(CREDENTIAL, mount, {})
  const stored = readTree(out.next_token)
  for (const [fid, e] of Object.entries(stored.m)) {
    assert.equal(e.t, null)
    assert.equal(e.u, `https://graph.microsoft.com/opaque/${fid}`)
    assert.equal(entryUrl(mount, fid, e), `https://graph.microsoft.com/opaque/${fid}`)
  }
})

test('a NEW or RENAMED folder emits the folder ITEM too, not just its messages', () => {
  // Without it the delta relocated the MESSAGES and left the folder NODE behind:
  // `ensure_ancestors` invented a plain raisin:Folder at the new chain carrying
  // no __external_id — so reconcile_deletes could never prune it — while the
  // real mount-owned folder node kept its old, now-empty path until the next
  // COMPLETE full walk. The user saw both.
  const mount = treeMount()
  stubRouter(mailboxRouter(FIXTURE))
  const first = opGetChanges(CREDENTIAL, mount, {})
  const walked = mailWalkPaths(mount, mailboxRouter(FIXTURE))

  // The first round emits every folder, and BYTE-IDENTICALLY to the walk — the
  // etag included, or one path would skip-write over the other's placement.
  stubRouter(mailboxRouter(FIXTURE))
  const walkItems = {}
  {
    const stack = [['inbox', '']]
    while (stack.length) {
      const [fid, prefix] = stack.pop()
      stubRouter(mailboxRouter(FIXTURE))
      for (const it of opList(CREDENTIAL, mount, { folder_id: fid, limit: 500 }).items) {
        const rel = prefix ? `${prefix}/${it.name}` : it.name
        if (it.is_folder) {
          walkItems[it.external_id] = it
          stack.push([it.external_id, rel])
        }
      }
    }
  }
  const deltaFolders = Object.fromEntries(
    first.items.filter((c) => c.item.is_folder).map((c) => [c.item.external_id, c])
  )
  assert.deepEqual(
    Object.keys(deltaFolders).sort(),
    ['F-ACME', 'F-NEWS', 'F-PROJ'],
    'every folder of the subtree arrives as a folder item; the mount ROOT does not'
  )
  for (const fid of Object.keys(deltaFolders)) {
    assert.deepEqual(deltaFolders[fid].item, walkItems[fid], `${fid}: the walk and the delta must emit ONE shape`)
    assert.equal(deltaFolders[fid].relative_path, walked[fid], `${fid}: and one path`)
  }

  // A RENAME re-emits the folder item at the new chain with a new etag, so the
  // folder node itself relocates rather than being stranded.
  const renamed = {
    ...FIXTURE,
    folders: FIXTURE.folders.map((f) => (f.id === 'F-PROJ' ? { ...f, displayName: 'Programs' } : f)),
  }
  stubRouter(mailboxRouter(renamed))
  const second = opGetChanges(CREDENTIAL, mount, { since_token: first.next_token })
  const moved = second.items.filter((c) => c.item.is_folder)
  const byId = Object.fromEntries(moved.map((c) => [c.item.external_id, c]))
  assert.equal(byId['F-PROJ'].relative_path, 'Programs')
  assert.equal(byId['F-PROJ'].item.etag, 'mailfolder-1|p=Programs')
  assert.notEqual(byId['F-PROJ'].item.etag, deltaFolders['F-PROJ'].item.etag)
  assert.equal(byId['F-ACME'].relative_path, 'Programs/Acme-Corp', 'and the child folder node moves with it')
  assert.equal(byId['F-NEWS'], undefined, 'a folder that did not move emits nothing')

  // A NEW folder arrives as a folder item on the delta as well.
  const grown = {
    ...FIXTURE,
    root: { ...FIXTURE.root, childFolderCount: 3 },
    folders: [
      ...FIXTURE.folders,
      { id: 'F-INV', displayName: 'Invoices', parentFolderId: 'F-INBOX', childFolderCount: 0 },
    ],
    messages: { ...FIXTURE.messages, 'F-INV': [] },
  }
  stubRouter(mailboxRouter(grown))
  const third = opGetChanges(CREDENTIAL, mount, { since_token: first.next_token })
  const born = third.items.find((c) => c.item.external_id === 'F-INV')
  assert.ok(born, 'a folder created at the provider becomes a mount-owned node')
  assert.equal(born.item.is_folder, true)
  assert.equal(born.relative_path, 'Invoices')
  assert.equal(born.item.parent_id, 'F-INBOX')
})

test('a mail-tree page cursor carries the chain, so a second page costs no lookups', () => {
  // mailTreeList re-resolved the mount root (one request) and re-walked the
  // folder's whole ancestor chain (one per level) on EVERY page. A folder three
  // levels down with ten pages of mail paid forty requests to re-learn a string
  // that cannot change between two pages of one listing.
  const paged = (url) => {
    if (url === 'https://graph/page2') return { body: { value: [msg('M-P2', 'F-ACME')] } }
    if (url.includes('/mailFolders/F-ACME/messages?')) {
      return { body: { value: [msg('M-P1', 'F-ACME')], '@odata.nextLink': 'https://graph/page2' } }
    }
    return mailboxRouter(FIXTURE)(url)
  }
  const mount = treeMount()

  let calls = stubRouter(paged)
  const page1 = opList(CREDENTIAL, mount, { folder_id: 'F-ACME', limit: 500 })
  assert.ok(page1.next_cursor, 'there is a second page')
  const stubFirst = calls

  calls = stubRouter(paged)
  const page2 = opList(CREDENTIAL, mount, { folder_id: 'F-ACME', cursor: page1.next_cursor, limit: 500 })
  const lookups = (cs) => cs.filter((c) => /\/mailFolders\/[^/?]+\?/.test(c.url)).length
  assert.equal(lookups(calls), 0, 'a resumed page resolves neither the root nor the chain again')
  assert.ok(lookups(stubFirst) >= 2, 'while the first page pays for the root AND the ancestor walk')
  // And the resumed page places its messages at the SAME chain, or the two pages
  // of one folder would materialize in two different places.
  assert.equal(page2.items[0].etag, 'W/"M-P2-1"|p=Projects/Acme-Corp')
  assert.equal(page1.items[0].etag, 'W/"M-P1-1"|p=Projects/Acme-Corp')
})

test('flipping folder_scope invalidates the cursor instead of replaying the old query', () => {
  // A folder-scoped link under a tree mount would sync one folder of N and
  // report healthy; a tree map under a folder mount is not a Graph link at all.
  const folderToken = mintCursor(mailMount())
  assert.throws(
    () => opGetChanges(CREDENTIAL, treeMount(), { since_token: folderToken }),
    /cursor_invalid|folder-tree/
  )

  stubRouter(mailboxRouter(FIXTURE))
  const treeToken = opGetChanges(CREDENTIAL, treeMount(), {}).next_token
  assert.throws(
    () => opGetChanges(CREDENTIAL, mailMount(), { since_token: treeToken }),
    /cursor identity|different query|predates/
  )
})

test('a truncated folder set is refused, never silently synced', () => {
  // A truncated listing is a PARTIAL `seen` set for the walk, and
  // reconcile_deletes would then prune every message in the folders that fell
  // off the end. Refusing is recoverable; deleting real content is not.
  const mount = treeMount({ sync_config: { resource: 'mail', folder_scope: 'tree', max_folders: 2 } })
  stubRouter(mailboxRouter(FIXTURE))
  assert.throws(() => opGetChanges(CREDENTIAL, mount, {}), /max_folders/)
})

test('a tree mount subscribes MAILBOX-WIDE, not to one folder out of N', () => {
  assert.equal(subscriptionResource(treeMount()), '/me/messages')
  assert.equal(subscriptionResource(mailMount()), '/me/mailFolders/inbox/messages')
  // A shared mailbox keeps its principal on both.
  assert.equal(
    subscriptionResource(treeMount({ sync_config: { resource: 'mail', folder_scope: 'tree', principal: 'sales@x.test' } })),
    '/users/sales%40x.test/messages'
  )
})

test('a folder name that is a path, or nothing at all, cannot escape the mount', () => {
  // The engine joins relative_path to mount_path VERBATIM.
  assert.equal(folderSegment('Acme/Corp', 'F-1'), 'Acme-Corp')
  assert.equal(folderSegment('..', 'F-1'), 'F-1')
  assert.equal(folderSegment('.', 'F-1'), 'F-1')
  assert.equal(folderSegment('   ', 'F-1'), 'F-1')
  assert.equal(folderSegment(null, 'F-1'), 'F-1')
  assert.equal(folderSegment('a\\b', 'F-1'), 'a-b')
})

test('hidden folders are enumerated, so their mail arrives from a feed we hold', () => {
  // Clutter and friends are excluded from a folder listing by default. Left
  // out, their messages arrive from no feed at all and are invisible with
  // nothing to observe.
  const calls = stubRouter(mailboxRouter(FIXTURE))
  opGetChanges(CREDENTIAL, treeMount(), {})
  const listings = calls.filter((c) => c.url.includes('/childFolders'))
  assert.ok(listings.length)
  for (const c of listings) assert.match(c.url, /includeHiddenFolders=true/)
})

test('an expired delta link reseeds THAT folder, never the whole tree cursor', () => {
  // Graph expires a mail delta token and answers 410 / syncStateNotFound, which
  // `http.js` maps to `cursor_invalid`. Let out of a tree mount that is a
  // WHOLE-MAILBOX RE-IMPORT: the engine recovers `cursor_invalid` by clearing
  // `last_sync_token` and running a full walk (`phases.rs`), so one of N links
  // aging out discards the N-1 that were fine and re-walks every folder and
  // every message in the subtree.
  const mount = treeMount()
  stubRouter(mailboxRouter(FIXTURE))
  const first = opGetChanges(CREDENTIAL, mount, {})
  const before = readTree(first.next_token).m
  assert.equal(before['F-NEWS'].s, 'delta', 'the fixture must start with live links')

  // F-PROJ's stored link is the only one Graph now rejects.
  const expired = (url) => {
    if (url.includes('F-PROJ') && url.includes('deltatoken')) {
      return {
        status: 410,
        body: { error: { code: 'syncStateNotFound', message: 'Sync state not found' } },
      }
    }
    return mailboxRouter(FIXTURE)(url)
  }
  stubRouter(expired)
  const second = opGetChanges(CREDENTIAL, mount, { since_token: first.next_token })
  const after = readTree(second.next_token).m

  assert.equal(after['F-PROJ'].s, 'enum', 'the rejected folder is reseeded with an enumeration')
  assert.equal(after['F-PROJ'].t, null, 'with NO token, which is what a fresh enumeration is')
  const reseeded = entryUrl(mount, 'F-PROJ', after['F-PROJ'])
  assert.match(reseeded, /\/mailFolders\/F-PROJ\/messages\/delta\?\$select=/)
  assert.doesNotMatch(reseeded, /deltatoken=latest/, 'and never with `latest`, or its backlog is lost')
  for (const fid of ['F-INBOX', 'F-NEWS', 'F-ACME']) {
    assert.equal(after[fid].t, before[fid].t, `${fid}'s still-valid token must survive`)
    assert.equal(after[fid].s, 'delta')
  }
  assert.equal(second.has_more, true, 'the rotation comes back to the reseeded folder')
})

test('the walk refuses a mailbox above max_folders, at the START of the backfill', () => {
  // buildFolderMap throws config_error above the ceiling, but only get_changes
  // called it — so an over-ceiling mailbox backfilled in full and only then
  // discovered, on its first delta, that it can never sync. The operator learned
  // about the limit at the one moment the expensive work was already done.
  const mount = treeMount({ sync_config: { resource: 'mail', folder_scope: 'tree', max_folders: 2 } })
  stubRouter(mailboxRouter(FIXTURE))
  assert.throws(() => opList(CREDENTIAL, mount, { folder_id: 'inbox', limit: 500 }), /max_folders/)

  // Once per WALK, not once per page or once per folder: a resumed page and a
  // subfolder pop pay nothing for the check.
  const roomy = treeMount()
  let calls = stubRouter(mailboxRouter(FIXTURE))
  opList(CREDENTIAL, roomy, { folder_id: 'F-PROJ', limit: 500 })
  const subtreeListings = calls.filter((c) => c.url.includes('/childFolders')).length
  const withPage2 = (url) =>
    url === 'https://graph/next' ? { body: { value: [] } } : mailboxRouter(FIXTURE)(url)
  calls = stubRouter(withPage2)
  opList(CREDENTIAL, roomy, { folder_id: 'inbox', cursor: 'https://graph/next', limit: 500 })
  assert.equal(
    calls.filter((c) => c.url.includes('/childFolders')).length <= subtreeListings,
    true,
    'a resumed page does not rebuild the folder map'
  )
})

test('a page cursor that wears our prefix but does not parse is never FETCHED as a url', () => {
  // `params.cursor` doubles as the raw Graph link (a bare link persisted by the
  // previous cursor shape still resumes), so a truncated `rsn-mailpage-1:` blob
  // would be handed to graphFetch verbatim and issue a request to a nonsense
  // host — one page of one folder silently lost, or a hard failure, depending
  // on what the http layer does with it.
  const mount = treeMount()
  const calls = stubRouter(mailboxRouter(FIXTURE))
  const page = opList(CREDENTIAL, mount, {
    folder_id: 'F-PROJ',
    cursor: 'rsn-mailpage-1:{"u":"https://graph/nex',
    limit: 500,
  })
  for (const c of calls) {
    assert.match(c.url, /^https:\/\/graph\.microsoft\.com\//, `fetched a non-url: ${c.url}`)
  }
  // It restarts this folder's listing rather than resuming a page it cannot
  // read; re-reading a page is idempotent through the etag skip-write.
  assert.deepEqual(page.items.map((i) => i.external_id).sort(), ['F-ACME', 'M-PROJ'])
})

// ---- the mail page bound --------------------------------------------------
//
// `max_items_per_sync` (500) was handed to Graph as `$top`. The engine's ITEM
// BUDGET FOR A RUN and the SIZE OF ONE RESPONSE are different quantities, and
// with `include_body` on the difference is fatal: one request asked for 500
// whole HTML documents, which the host buffers, parses into a serde_json::Value
// and materializes again as QuickJS objects inside a 64 MB heap. The adapter
// died with `out of memory at graphFetch` before it ever saw the page; the
// engine reads an OOM as transient and retried the identical request forever,
// and a non-zero failure counter disables the back-to-back backfill, so the
// import froze at the item count of the last page that happened to fit.

function topOf(url) {
  const m = /[?&]\$?(?:%24)?top=(\d+)/.exec(url)
  return m ? Number(m[1]) : null
}

test('a mail page is bounded by what the RESPONSE weighs, not by the run budget', () => {
  for (const limit of [1, 100, 5000]) {
    const lean = stubHttp([{ body: { value: [] } }])
    opList(CREDENTIAL, mailMount(), { limit })
    assert.equal(
      topOf(lean[0].url),
      Math.min(limit, MAIL_PAGE),
      `lean $select at limit=${limit}: the run budget may only ever LOWER the page`
    )

    const heavy = stubHttp([{ body: { value: [] } }])
    opList(CREDENTIAL, mailMount({ sync_config: { resource: 'mail', include_body: true } }), { limit })
    assert.equal(
      topOf(heavy[0].url),
      Math.min(limit, MAIL_BODY_PAGE),
      `include_body at limit=${limit}: a body is an unbounded HTML document, ` +
        'so the page has to be an order of magnitude smaller'
    )
  }
})

test('the mail TREE walk pays the same bound as the flat listing', () => {
  // The tree walk has its own listing shape (`mailTreeList`), so it is its own
  // call site and would otherwise keep passing the budget through untouched —
  // and it is the path that meets the oversized page FIRST, because it issues a
  // list call per folder rather than one for the whole mount.
  const mount = treeMount({
    sync_config: { resource: 'mail', folder_scope: 'tree', include_body: true },
  })
  const calls = stubRouter(mailboxRouter(FIXTURE))
  opList(CREDENTIAL, mount, { folder_id: 'F-PROJ', limit: 500 })
  const listing = calls.find((c) => c.url.includes('/messages'))
  assert.equal(topOf(listing.url), MAIL_BODY_PAGE)
})

test('the mail delta sends a $top rather than letting Graph choose the page', () => {
  // Without one Graph picks, and a page Graph considers reasonable is — with
  // bodies — the same stack of HTML documents that killed the walk. The delta
  // is what runs once the backfill finishes, so an unbounded feed just moves
  // the failure a few hours later.
  const mount = mailMount({ sync_config: { resource: 'mail', include_body: true } })
  const calls = stubHttp([{ body: { value: [], '@odata.deltaLink': 'https://graph/d?$deltatoken=T' } }])
  opGetChanges(CREDENTIAL, mount, {})
  assert.equal(topOf(calls[0].url), MAIL_BODY_PAGE)
})

test('sync_config.page_size lowers the ceiling but never raises it past the budget', () => {
  const mount = mailMount({ sync_config: { resource: 'mail', page_size: 7 } })
  const calls = stubHttp([{ body: { value: [] } }])
  opList(CREDENTIAL, mount, { limit: 500 })
  assert.equal(topOf(calls[0].url), 7)

  // A ceiling, not a floor: the run's own budget still wins when it is smaller,
  // so raising this can never make the engine stage more than it asked for.
  const roomy = mailMount({ sync_config: { resource: 'mail', page_size: 900 } })
  const small = stubHttp([{ body: { value: [] } }])
  opList(CREDENTIAL, roomy, { limit: 3 })
  assert.equal(topOf(small[0].url), 3)
})

test('a page cursor from the PREVIOUS version is retired, not fetched as a url', () => {
  // Bounding `$top` does nothing for a mount already part-way through a
  // listing: Graph freezes the page size into the `nextLink` that minted it, so
  // a stored cursor keeps re-fetching the oversized page and keeps running out
  // of memory. Versioning the cursor is what unwedges it — but only if the
  // guard matches the FAMILY prefix. Matched against the CURRENT version alone,
  // a well-formed v1 blob stops looking like one of ours, falls through as the
  // raw URL and is handed to graphFetch verbatim.
  const mount = treeMount()
  const calls = stubRouter(mailboxRouter(FIXTURE))
  const page = opList(CREDENTIAL, mount, {
    folder_id: 'F-PROJ',
    cursor: 'rsn-mailpage-1:' + JSON.stringify({ u: 'https://graph/next', r: 'F-INBOX', p: 'Projects' }),
    limit: 500,
  })
  for (const c of calls) {
    assert.match(c.url, /^https:\/\/graph\.microsoft\.com\//, `fetched a non-url: ${c.url}`)
  }
  // Restarted under the new bound rather than resumed under the old one.
  assert.deepEqual(page.items.map((i) => i.external_id).sort(), ['F-ACME', 'M-PROJ'])
  assert.equal(topOf(calls.find((c) => c.url.includes('/messages')).url), MAIL_PAGE)
})
