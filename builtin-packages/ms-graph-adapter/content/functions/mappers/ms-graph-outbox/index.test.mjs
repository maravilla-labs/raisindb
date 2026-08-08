// SPDX-License-Identifier: BSL-1.1
//
// Outbox-mapper tests. Run with `node --test index.test.mjs`.
//
// This mapper had NO tests, and it is the highest-consequence surface in the
// adapter: it decides what an irreversible, externally-visible send actually
// contains. A bug here is a wrong email in a stranger's inbox, a meeting
// response nobody meant to give, or a silent refusal to send at all.
//
// The rule the assertions below encode: when this mapper is unsure, it returns
// NULL. The engine records "the mapper declined", the command stays pending and
// a human can act on it — whereas a guess SENDS something.

import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

// Loaded the way the engine loads it: a bare script whose entry point is the
// global `handler`.
const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8')
const handler = new Function(`${src}\nreturn handler;`)()

const MOUNT = { mount_id: 'm1', mount_path: '/outbox', sync_config: {} }

const out = (properties, mount = MOUNT) =>
  handler({ operation: 'to_external', node: { properties }, mount })

// ---- shape -----------------------------------------------------------------

test('the outbox writes outward only and never imports', () => {
  assert.deepEqual(handler({ operation: 'mapper_capabilities', mount: MOUNT }), {
    to_external: true,
  })
  // to_node returns null on purpose: importing from an outbox would
  // materialize sent mail as new commands to send.
  assert.equal(handler({ operation: 'to_node', external_item: { external_id: 'x' } }), null)
  assert.equal(handler({ external_item: { external_id: 'x' } }), null)
  assert.equal(handler({ operation: 'to_external', node: null, mount: MOUNT }), null)
})

// ---- send ------------------------------------------------------------------

test('a send carries exactly the message that was composed', () => {
  const r = out({
    action: 'send',
    subject: 'Q3 numbers',
    body_html: '<p>Attached.</p>',
    to: ['a@example.com', { address: 'b@example.com', name: 'Bee' }],
    cc: ['c@example.com'],
    bcc: ['d@example.com'],
    importance: 'high',
  })
  assert.equal(r.payload.action, 'send')
  const m = r.payload.body.message
  assert.equal(m.subject, 'Q3 numbers')
  assert.deepEqual(m.body, { contentType: 'HTML', content: '<p>Attached.</p>' })
  assert.deepEqual(m.toRecipients, [
    { emailAddress: { address: 'a@example.com' } },
    { emailAddress: { address: 'b@example.com', name: 'Bee' } },
  ])
  assert.equal(m.ccRecipients.length, 1)
  assert.equal(m.bccRecipients.length, 1)
  assert.equal(m.importance, 'high')
})

test('HTML wins over text, because Graph takes one contentType', () => {
  // Sending the text half of a message someone composed in HTML is a silent
  // downgrade nobody sees until the recipient does.
  const r = out({
    action: 'send',
    to: ['a@example.com'],
    body_html: '<b>rich</b>',
    body_text: 'plain',
  })
  assert.deepEqual(r.payload.body.message.body, { contentType: 'HTML', content: '<b>rich</b>' })

  const textOnly = out({ action: 'send', to: ['a@example.com'], body_text: 'plain' })
  assert.deepEqual(textOnly.payload.body.message.body, { contentType: 'Text', content: 'plain' })
})

test('a send with nowhere to go is declined, not attempted', () => {
  assert.equal(out({ action: 'send', subject: 'no recipients' }), null)
  // An unusable recipient entry is dropped; if that empties the list, the whole
  // send is declined rather than issued with an empty toRecipients (which Graph
  // rejects for the entire message).
  assert.equal(out({ action: 'send', to: [{ name: 'No Address' }, null] }), null)
})

test('recipients tolerate both spellings a real app produces', () => {
  const r = out({
    action: 'send',
    to: ['bare@example.com', { address: 'obj@example.com' }, { email: 'alt@example.com' }],
  })
  assert.deepEqual(
    r.payload.body.message.toRecipients.map((x) => x.emailAddress.address),
    ['bare@example.com', 'obj@example.com', 'alt@example.com']
  )
})

// ---- reply / forward -------------------------------------------------------

test('a reply must name the message it answers', () => {
  for (const action of ['reply', 'reply_all', 'forward']) {
    assert.equal(out({ action, body_text: 'hi' }), null, `${action} without a target must decline`)
  }
})

test('a reply targets the original and inherits its recipients', () => {
  const r = out({ action: 'reply', in_reply_to_external_id: 'AAMkAGI2', body_text: 'thanks' })
  assert.equal(r.payload.action, 'reply')
  // The engine addresses the ORIGINAL message; Graph derives the recipients.
  assert.equal(r.external_id, 'AAMkAGI2')
  assert.equal(r.payload.body.message.body.content, 'thanks')

  const all = out({ action: 'reply_all', in_reply_to_external_id: 'AAMkAGI2', body_text: 'x' })
  assert.equal(all.payload.action, 'reply_all')
})

test('a forward with no recipients is declined — unlike a reply, it inherits none', () => {
  assert.equal(out({ action: 'forward', in_reply_to_external_id: 'AAMkAGI2', body_text: 'fyi' }), null)
  const ok = out({
    action: 'forward',
    in_reply_to_external_id: 'AAMkAGI2',
    to: ['x@example.com'],
    body_text: 'fyi',
  })
  assert.equal(ok.payload.action, 'forward')
  assert.equal(ok.external_id, 'AAMkAGI2')
})

// ---- calendar RSVP ---------------------------------------------------------

test('an RSVP must name the event it answers', () => {
  // Without a target there is no event to respond to, and responding to the
  // wrong one is worse than not responding.
  for (const action of ['accept', 'decline', 'tentative']) {
    assert.equal(out({ action, comment: 'yes' }), null)
  }
})

test('an RSVP sends a response by default and can be told not to', () => {
  const r = out({ action: 'accept', target_external_id: 'EVT-1', comment: 'see you there' })
  assert.equal(r.payload.action, 'accept')
  assert.equal(r.external_id, 'EVT-1')
  assert.equal(r.payload.body.comment, 'see you there')
  // Default TRUE, and stated EXPLICITLY rather than left to a provider default
  // that could change: an RSVP the organizer never receives is not an RSVP.
  assert.equal(r.payload.body.sendResponse, true)

  const quiet = out({ action: 'decline', target_external_id: 'EVT-1', send_response: false })
  assert.equal(quiet.payload.body.sendResponse, false)

  for (const action of ['accept', 'decline', 'tentative']) {
    assert.equal(out({ action, target_external_id: 'EVT-1' }).payload.action, action)
  }
})

// ---- the refusal contract --------------------------------------------------

test('an unknown or missing action is declined rather than guessed', () => {
  // A guess here SENDS something. Null leaves the command pending with a
  // recorded reason a human can act on.
  assert.equal(out({ action: 'launch_missiles', to: ['a@example.com'] }), null)
  assert.equal(out({ subject: 'no action at all' }), null)
  assert.equal(out({}), null)
})
