// Outbox-mapper tests. Run with `node --test index.test.mjs`.
//
// This mapper decides what an irreversible, externally-visible send actually
// contains. A bug here is a wrong email in a stranger's inbox, or a silent
// refusal to send at all.
//
// The rule every assertion encodes: when this mapper is unsure, it returns
// NULL. The engine records "the mapper declined", the command settles as failed
// with a stated reason and a human can act on it — whereas a guess SENDS
// something.

import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

// Loaded the way the engine loads it: a bare script whose entry point is the
// global `handler`.
const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8')
const handler = new Function(`${src}\nreturn handler;`)()

const MOUNT = { mount_id: 'm1', mount_path: '/mail/outbox', sync_config: {} }

const out = (properties) =>
  handler({ operation: 'to_external', node: { properties }, mount: MOUNT })

const SENDABLE = {
  action: 'send',
  to: ['ada@example.org'],
  subject: 'Hello',
  body_text: 'Hi there',
}

// ---- shape -----------------------------------------------------------------

test('the outbox writes outward only and never imports', () => {
  assert.deepEqual(handler({ operation: 'mapper_capabilities', mount: MOUNT }), {
    to_external: true,
  })
  // Without this the engine refuses EVERY write mode for the mount, however the
  // adapter's capabilities are declared: writability belongs to adapter and
  // mapper together.

  // to_node returns null on purpose: importing from an outbox would materialize
  // sent mail as fresh commands to send.
  assert.equal(handler({ operation: 'to_node', external_item: { external_id: 'x' } }), null)
  assert.equal(handler({ external_item: { external_id: 'x' } }), null)
  assert.equal(handler({ operation: 'to_external', node: null, mount: MOUNT }), null)
  assert.equal(handler(null), null)
})

// ---- the message -----------------------------------------------------------

test('a sendable command becomes one raisin.email.send message', () => {
  const mapped = out({
    ...SENDABLE,
    cc: ['bob@example.org'],
    bcc: ['audit@example.org'],
    body_html: '<p>Hi there</p>',
  })
  assert.deepEqual(mapped, {
    payload: {
      action: 'send',
      body: {
        to: ['ada@example.org'],
        subject: 'Hello',
        text: 'Hi there',
        html: '<p>Hi there</p>',
        cc: ['bob@example.org'],
        bcc: ['audit@example.org'],
      },
    },
  })
})

test('the provider is not the mapper business, and neither is the sender', () => {
  const body = out(SENDABLE).payload.body
  // Which configured sender this mount posts through is a CONNECTION setting the
  // adapter adds from mount config; putting it on every command node would let
  // two mounts of the same outbox disagree.
  assert.equal('provider' in body, false)
  // And a function never chooses who a message appears to be FROM: `from` comes
  // from the tenant's /config/email entry, so there is no field for it here.
  assert.equal('from' in body, false)
})

test('recipients are bare addresses, however they were written', () => {
  const body = out({
    ...SENDABLE,
    to: ['ada@example.org', { address: 'bob@example.org', name: 'Bob' }, { email: 'c@x.test' }],
  }).payload.body
  // The display name is dropped, unlike the Graph mapper: the email API takes
  // addresses only, and "Bob <bob@example.org>" in an address slot fails its
  // validation as one string.
  assert.deepEqual(body.to, ['ada@example.org', 'bob@example.org', 'c@x.test'])

  // A single address is the spelling a compose UI reaches for first.
  assert.deepEqual(out({ ...SENDABLE, to: 'ada@example.org' }).payload.body.to, [
    'ada@example.org',
  ])

  // An entry with no address is DROPPED rather than sent as an empty recipient,
  // which is refused for the whole message.
  assert.deepEqual(out({ ...SENDABLE, to: ['ada@example.org', { name: 'nobody' }, ''] }).payload
    .body.to, ['ada@example.org'])
})

test('an HTML-only body still gets a text alternative', () => {
  // raisin.email.send REQUIRES a non-empty `text` and refuses an HTML-only
  // message ([email:invalid_message] "empty text body") — so every command
  // composed in a rich editor would fail at drain time, naming a field the
  // author never saw.
  const body = out({
    action: 'send',
    to: ['ada@example.org'],
    subject: 'Hello',
    body_html: '<style>p{}</style><p>Hi&nbsp;<b>there</b></p><br><p>Bye &amp; thanks</p>',
  }).payload.body
  assert.equal(body.text, 'Hi there\n\nBye & thanks')
  assert.equal(body.html.startsWith('<style>'), true)
})

test('an explicit text body is never overwritten by the HTML rendering', () => {
  const body = out({ ...SENDABLE, body_html: '<p>something else</p>' }).payload.body
  assert.equal(body.text, 'Hi there')
})

// ---- declining -------------------------------------------------------------

test('only `send` is mapped; reply and forward are declined, not approximated', () => {
  // Threading them needs In-Reply-To/References on the outgoing message, and the
  // email API accepts no headers. A "reply" that is really a fresh message with
  // a Re: subject breaks the recipient's thread silently.
  for (const action of ['reply', 'reply_all', 'forward', 'rsvp', '']) {
    assert.equal(out({ ...SENDABLE, action }), null, action)
  }
  assert.equal(out({ to: ['a@x.test'], subject: 'x', body_text: 'y' }), null)
})

test('a command that could not be sent as written is declined here, not at the provider', () => {
  // No recipient, no subject, no body: each is refused by the email API's own
  // validation, and declining locally keeps the reason "this command is not
  // sendable" instead of a provider validation error against a command the
  // engine has already claimed.
  assert.equal(out({ ...SENDABLE, to: [] }), null)
  assert.equal(out({ ...SENDABLE, to: undefined }), null)
  assert.equal(out({ ...SENDABLE, subject: '' }), null)
  assert.equal(out({ ...SENDABLE, body_text: '', body_html: '' }), null)
  // An HTML body that renders to nothing is no body at all.
  assert.equal(out({ ...SENDABLE, body_text: '', body_html: '<div>  </div>' }), null)
})

test('what is not carried is absent, not half-sent', () => {
  const body = out({ ...SENDABLE, importance: 'high', attachments: [{ name: 'x.pdf' }] })
    .payload.body
  // The message shape has no `importance`, and an attachment must name a source
  // the server can read — the raisin:Asset children an inbound IMAP message maps
  // to carry no `file` by design, so there is nothing to point at yet.
  assert.equal('importance' in body, false)
  assert.equal('attachments' in body, false)
})
