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

import { opList } from './read.js'
import { opGetChanges } from './changes.js'
import { raiseForStatus } from './http.js'
import { outlookHeaders, useImmutableIds } from './mount.js'

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

test('has_more distinguishes a mid-enumeration page from a caught-up cursor', () => {
  // A nextLink means "more pages right now".
  stubHttp([{ body: { value: [], '@odata.nextLink': 'https://graph/next' } }])
  const paging = opGetChanges(CREDENTIAL, mailMount(), { since_token: 't0' })
  assert.equal(paging.has_more, true)
  assert.equal(paging.next_token, 'https://graph/next')

  // A deltaLink means "caught up — this token is the NEXT RUN's resume point".
  // Reporting has_more here is what span the loop against an idle feed.
  stubHttp([{ body: { value: [], '@odata.deltaLink': 'https://graph/delta' } }])
  const caught = opGetChanges(CREDENTIAL, mailMount(), { since_token: 't0' })
  assert.equal(caught.has_more, false)
  assert.equal(caught.next_token, 'https://graph/delta')
})

test('get_changes never returns a null cursor', () => {
  // Null reads to the engine as "no resumable cursor exists" — the stored
  // cursor must survive a page that carries neither link.
  stubHttp([{ body: { value: [] } }])
  const out = opGetChanges(CREDENTIAL, mailMount(), { since_token: 'keep-me' })
  assert.equal(out.next_token, 'keep-me')
  assert.equal(out.has_more, false)
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
