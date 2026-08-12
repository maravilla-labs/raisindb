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

import test from 'node:test'
import assert from 'node:assert/strict'

import { opList, opGetContent } from './read.js'
import { opGetChanges } from './changes.js'
import { raiseForStatus } from './http.js'
import { outlookHeaders, useImmutableIds } from './mount.js'
import { opUpdate } from './write.js'
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
