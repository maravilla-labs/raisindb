// Adapter-level tests for the SEND path. Run with `node --test index.test.mjs`.
//
// IMAP cannot send, so a `submit` mount sends through the tenant's configured
// email provider instead. Everything asserted here guards one of the two ways
// that arrangement goes wrong:
//
//  * a capability declared with nothing behind it — the mount resolves as
//    writable, the engine claims the command, and the drain throws. So
//    can_submit is false unless a sender is resolvable RIGHT NOW, including in
//    the cases that look configured from a distance (email switched off, the
//    named entry disabled, several enabled entries and no default).
//  * a failed send that is retried when it may already have been delivered.
//    Only a refusal that proves nothing left may requeue; everything ambiguous
//    is parked.

import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

// Loaded the way the engine loads it: a bare script whose entry point is the
// global `handler`.
const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8')
const handler = new Function(`${src}\nreturn handler;`)()

/** Stub raisin.email; records every send. */
function stubEmail({ providers, sendResult, sendThrows }) {
  const sent = []
  globalThis.raisin = {
    email: {
      providers() {
        if (providers instanceof Error) throw providers
        return providers
      },
      send(message) {
        sent.push(message)
        if (sendThrows) throw sendThrows
        return sendResult === undefined ? { message_id: 'mid-1', provider: 'smtp' } : sendResult
      },
    },
  }
  return sent
}

const ONE_DEFAULT = {
  enabled: true,
  providers: [
    { name: 'relay', provider: 'smtp', from_address: 'noreply@example.org', enabled: true, default: true },
  ],
}

const mount = (sync = {}) => ({ mount_id: 'm1', mount_path: '/mail', sync_config: sync })

const caps = (m = mount()) => handler({ operation: 'capabilities', mount: m })

const submit = (body, m = mount(), action = 'send') =>
  handler({
    operation: 'submit',
    mount: m,
    params: { payload: { action, body }, idempotency_key: 'm1:n1:a1' },
  })

const MESSAGE = { to: ['ada@example.org'], subject: 'Hi', text: 'body' }

// ---- capabilities ----------------------------------------------------------

test('can_submit is declared only when a sender actually resolves', () => {
  stubEmail({ providers: ONE_DEFAULT })
  const c = caps()
  assert.equal(c.can_submit, true)
  // The engine's missing_submit_ops demands can_write alongside it: an adapter
  // that sets can_submit without it has contradicted itself.
  assert.equal(c.can_write, true)
  // …and it is NOT a mirror. A mirror or state_only mount must still be refused,
  // naming these.
  assert.equal(c.can_create, undefined)
  assert.equal(c.can_update, undefined)
  assert.equal(c.can_delete, undefined)
  // No provider-side idempotency key exists for SMTP, so at-most-once rests
  // entirely on the engine's durable claim.
  assert.equal(c.supports_idempotency_key, false)
  assert.equal(c.submit_unavailable_reason, undefined)
})

test('reading is unaffected by the send path', () => {
  stubEmail({ providers: new Error('[email:policy_denied] no email_policy') })
  const c = caps()
  assert.equal(c.can_read, true)
  assert.equal(c.supports_changes, true)
  assert.equal(c.default_ttl, 86400)
  // The probe threw and the capabilities call still returned — a mount that
  // only reads must never be broken by the outbox half being unconfigured.
  assert.equal(c.can_submit, undefined)
})

test('the four unbacked-provider cases all report can_submit false, with the reason', () => {
  const cases = [
    // Denied by the function's own email_policy: the default, deny.
    [new Error('[email:policy_denied] this function has no email_policy'), /email_policy/],
    // Configured but switched off — indistinguishable from "nothing configured"
    // in a failed send, which is why it is diagnosed here.
    [{ enabled: false, providers: [{ name: 'relay', enabled: true, default: true }] }, /switched off/],
    // Every entry parked.
    [{ enabled: true, providers: [{ name: 'relay', enabled: false }] }, /no enabled email provider/],
    // The ambiguous one: two live senders, no default. `send` would throw at
    // drain time, after the engine had claimed the command.
    [
      {
        enabled: true,
        providers: [
          { name: 'a', enabled: true },
          { name: 'b', enabled: true },
        ],
      },
      /none is the default/,
    ],
  ]
  for (const [providers, reason] of cases) {
    stubEmail({ providers })
    const c = caps()
    assert.equal(c.can_submit, undefined, `${reason} should not declare can_submit`)
    assert.equal(c.can_write, false)
    assert.match(c.submit_unavailable_reason, reason)
  }
})

test('a mount naming an unknown or disabled provider does not resolve', () => {
  stubEmail({ providers: ONE_DEFAULT })
  assert.match(
    caps(mount({ email_provider: 'marketing' })).submit_unavailable_reason,
    /has not configured/
  )

  stubEmail({
    providers: { enabled: true, providers: [{ name: 'relay', enabled: false }] },
  })
  assert.match(caps(mount({ email_provider: 'relay' })).submit_unavailable_reason, /disabled/)
})

test('an entry with no from_address does not resolve, in either selection path', () => {
  // `EmailConfig::resolve` finishes with `sender.validate()`, whose first check
  // is a non-empty from_address — so an entry without one looks selectable and
  // then refuses the send with [email:config]. The listing carries the field,
  // so the probe can see it, and a capability that resolves and then throws is
  // exactly what this adapter must not declare.
  stubEmail({ providers: { enabled: true, providers: [{ name: 'relay', enabled: true, default: true }] } })
  assert.equal(caps().can_submit, undefined)
  assert.match(caps().submit_unavailable_reason, /no from_address/)

  // Same when the mount names it, rather than inheriting the default.
  stubEmail({
    providers: {
      enabled: true,
      providers: [{ name: 'relay', from_address: '   ', enabled: true }],
    },
  })
  assert.match(caps(mount({ email_provider: 'relay' })).submit_unavailable_reason, /no from_address/)
})

test('a credential the function may not read is terminal, not an ambiguous park', () => {
  // Resolving the sender's credential_ref runs the function's SECRET policy,
  // and providers() carries no credential_ref — so this refusal is the one the
  // capabilities probe cannot pre-empt. It happens before the socket, so the
  // command is definitively unsent: `failed`, which a person fixes and
  // requeues. Parked at `unknown` it would send someone to search a Sent folder
  // for a mail that was never attempted.
  stubEmail({
    providers: ONE_DEFAULT,
    sendThrows: new Error(
      "[secrets:policy_denied] cannot read secret 'email/api_key': not matched by this " +
        "function's secret_policy.allowed_names"
    ),
  })
  assert.throws(() => submit(MESSAGE), (e) => e.code === 'config_error')
})

test('a single enabled provider with no default flag is still unambiguous', () => {
  stubEmail({ providers: { enabled: true, providers: [{ name: 'only', from_address: 'a@example.org', enabled: true }] } })
  assert.equal(caps().can_submit, true)
})

// ---- submit ----------------------------------------------------------------

test('one queued command becomes one send, and the body is forwarded verbatim', () => {
  const sent = stubEmail({ providers: ONE_DEFAULT })
  const out = submit({ ...MESSAGE, cc: ['bob@example.org'], html: '<p>body</p>' })
  assert.equal(sent.length, 1)
  assert.deepEqual(sent[0], {
    to: ['ada@example.org'],
    subject: 'Hi',
    text: 'body',
    cc: ['bob@example.org'],
    html: '<p>body</p>',
  })
  // The receipt is acceptance, not delivery; the engine stores message_id as
  // the command's external id.
  assert.deepEqual(out, { external_id: 'mid-1', etag: null })
})

test('the mount names which configured sender to post through', () => {
  const sent = stubEmail({
    providers: {
      enabled: true,
      providers: [
        { name: 'transactional', from_address: 'tx@example.org', enabled: true, default: true },
        { name: 'Support', from_address: 'support@example.org', enabled: true },
      ],
    },
  })
  submit(MESSAGE, mount({ email_provider: 'support' }))
  // Matched case-insensitively, but sent with the CONFIGURED spelling.
  assert.equal(sent[0].provider, 'Support')
})

test('no email_provider omits the key rather than sending a null provider', () => {
  const sent = stubEmail({ providers: ONE_DEFAULT })
  submit(MESSAGE)
  // Omitting is how the email API is told "the tenant default"; a null would be
  // read as a provider name, and an unknown name is an error there.
  assert.equal('provider' in sent[0], false)
})

test('a send is refused before the API when the sender stopped resolving', () => {
  // The operator disabled the provider between the capabilities probe and the
  // drain. Terminal, and nothing was sent.
  const sent = stubEmail({
    providers: { enabled: true, providers: [{ name: 'relay', enabled: false }] },
  })
  assert.throws(() => submit(MESSAGE), (e) => e.code === 'config_error')
  assert.equal(sent.length, 0)
})

test('only `send` is issued; reply, forward and an empty body are terminal', () => {
  const sent = stubEmail({ providers: ONE_DEFAULT })
  for (const action of ['reply', 'reply_all', 'forward', 'accept']) {
    assert.throws(
      () => submit(MESSAGE, mount(), action),
      (e) => e.code === 'config_error' && /In-Reply-To/.test(e.message),
      action
    )
  }
  assert.throws(() => submit(null), (e) => e.code === 'config_error')
  assert.throws(() => submit({ subject: 'Hi' }), (e) => e.code === 'config_error')
  assert.throws(
    () => handler({ operation: 'submit', mount: mount(), params: { payload: {} } }),
    (e) => e.code === 'config_error'
  )
  assert.equal(sent.length, 0)
})

test('a refusal that proves nothing left is terminal, not a retry', () => {
  // Every one of these is decided before a socket is opened, or by the provider
  // before it looks at the message. `config_error` is the engine's `failed`: a
  // person edits and requeues.
  for (const tag of [
    '[email:policy_denied]',
    '[email:config]',
    '[email:invalid_message]',
    '[email:unsupported]',
    '[email:auth_failed]',
  ]) {
    stubEmail({ providers: ONE_DEFAULT, sendThrows: new Error(`${tag} nope`) })
    assert.throws(() => submit(MESSAGE), (e) => e.code === 'config_error', tag)
  }
})

test('a throttled send may be requeued; an ambiguous one is parked, never resent', () => {
  // 429 is the provider refusing to LOOK at the request — the only answer that
  // proves nothing was sent, and so the only one the engine may resend.
  stubEmail({ providers: ONE_DEFAULT, sendThrows: new Error('[email:rate_limited] slow down') })
  assert.throws(() => submit(MESSAGE), (e) => e.code === 'rate_limited')

  // A relay timeout most often means the message DID reach the relay and the
  // acknowledgement was lost. An uncoded Error reaches the engine as Transient,
  // which the submit drain parks at `unknown`: attributable, never auto-retried.
  // Resending here delivers a second copy to a real person.
  for (const tag of ['[email:transport]', '[email:timeout]', '[email:provider_error]']) {
    stubEmail({ providers: ONE_DEFAULT, sendThrows: new Error(`${tag} boom`) })
    assert.throws(
      () => submit(MESSAGE),
      (e) => e.code === undefined && /UNKNOWN/.test(e.message),
      tag
    )
  }
})

test('a provider that returns no message id still answers with an object', () => {
  // A null answer is read by the engine as "the outcome is unknown" and parks
  // the command; an absent id is not the same thing as an unknown outcome.
  stubEmail({ providers: ONE_DEFAULT, sendResult: {} })
  assert.deepEqual(submit(MESSAGE), { external_id: null, etag: null })
})

// ---- the mirror surface stays closed ---------------------------------------

test('create, update and delete are still unsupported', () => {
  stubEmail({ providers: ONE_DEFAULT })
  for (const operation of ['create', 'update', 'delete']) {
    assert.throws(
      () => handler({ operation, mount: mount(), params: {} }),
      // Uncoded: not a config error the operator can fix, just an operation
      // this connector does not have.
      (e) => e.code === undefined && /read protocol/.test(e.message),
      operation
    )
  }
})
