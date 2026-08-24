# Outbound email

Transactional mail — magic-link sign-in, notifications, receipts. Each tenant
sends through **their own** provider accounts from **their own** verified
domains.

There is deliberately no platform fallback sender. A tenant that has configured
nothing cannot send at all, rather than sending as somebody else.

## The two halves

Configuration is split, and the split is the point: the settings are a node (so
they are versioned, auditable and diffable), while the credentials are not (so
they are never in a node property, a revision, or a diff).

| | Where | Set via |
|---|---|---|
| Settings | `raisin:EmailConfig` node at `/config/email` in the `raisin:system` workspace | **Email** page in the admin console |
| API keys / SMTP passwords | secret store, one per provider, named by its `credential_ref` | **Secrets** page |

Both are per repository **branch** — the node lives in a branch's workspace and
the secret store is keyed `{tenant, repo, branch}`. A branch forked after the
config exists inherits both.

## Several providers, one default

A tenant configures **one or more** senders and marks **one default**.

```yaml
enabled: true
base_url: https://app.example.com
default_provider: transactional
providers:
  - name: transactional          # what a function names
    provider: resend             # which API it talks to
    from_address: no-reply@example.com
    from_name: Example
    credential_ref: secret://email/resend_api_key
  - name: relay
    provider: smtp
    from_address: no-reply@example.com
    credential_ref: secret://email/smtp_password
    smtp:
      host: smtp-relay.brevo.com
      port: 587
      username: account@example.com
      security: starttls
```

`name` is **not** the provider API: two Resend accounts are two entries with the
same `provider` and different names.

### Which one a send uses

One implementation answers this — `EmailConfig::resolve` — so the console, the
send path and anything added later cannot disagree about which account is the
default.

1. A send that **names** a provider gets that one. It must exist and be enabled;
   an unknown name is an **error**, never a fall-through to the default. Mail
   leaving through the wrong account is worse than mail not leaving.
2. A send that names none gets `default_provider`, else the entry flagged
   `default: true`, else the only enabled entry.
3. Several enabled entries and no default is a configuration error, for the same
   reason as (1).

`enabled` on the config outranks all of it: off means nothing sends, however
many providers are configured.

### Fields

| Field | Notes |
|---|---|
| `enabled` | Master switch, **defaults to false** |
| `base_url` | Absolute URL of the tenant's front end — magic links are built from it |
| `providers[]` | The configured senders (below) |
| `default_provider` | Name of the entry system mail goes through |
| `redirect_allowlist` | Redirect targets a magic-link verify may land on |

Per provider entry:

| Field | Notes |
|---|---|
| `name` | Unique slug a function names |
| `provider` | `resend`, `brevo` or `smtp` |
| `from_address` | Must be on a domain verified with **that entry's** account |
| `from_name`, `reply_to` | Optional |
| `credential_ref` | `secret://…`; a reference, never a key |
| `api_base` | Optional API base override (HTTP providers) |
| `smtp` | `{ host, port, username, security }` — required for `smtp` |
| `enabled` | Per-entry switch, **defaults to true** |
| `default` | Marks the default; `default_provider` wins when both are set |

## Sending

One `ApiMethodDescriptor` serves both runtimes, so QuickJS and Starlark see the
same call and the same behaviour:

```js
// QuickJS — the tenant default (what a magic link uses)
await raisin.email.send({ to: ["a@example.com"], subject: "Hi", text: "..." })

// …or a named provider
await raisin.email.send({ to: "a@example.com", subject: "Hi", text: "...",
                          provider: "marketing" })

// what is configured, with no credential in the answer
const { enabled, providers } = await raisin.email.providers()
```

```python
# Starlark
raisin.email.send({"to": ["a@example.com"], "subject": "Hi", "text": "..."})
```

A function chooses **which** configured account to use — never **who** it is.
`from`, `from_name` and `reply_to` come from the config, so a compromised
function cannot send as an address the tenant never verified.

The receipt is `{ message_id, provider, sender }`: `provider` is the API,
`sender` is the configured name it went through. Acceptance is not delivery.

## Three refusals, in the order they happen

Worth knowing in this order, because each one produces a different error and
looking at the wrong layer wastes an afternoon.

1. **The function's `email_policy`.** Deny-by-default, per function, declared in
   its `.node.yaml`:

   ```yaml
   email_policy:
     enabled: true
     allowed_recipients: ["*@example.com"]
   ```

   This runs *first* — before the config is read and before any credential is
   decrypted — so a function that was never granted email learns it from the
   policy rather than from a provider error, and a denied send never causes a
   key to be decrypted.

   Every recipient must be allowed. There is no partial send: a message to one
   permitted and one forbidden address is refused whole, because silently
   dropping a recipient would let the caller believe it went somewhere it did
   not.

   As with `secret_policy`, `enabled: true` with an empty `allowed_recipients`
   still denies everything. "Opted in" never silently means "unrestricted".

   **Mail to your own users needs `*`, and that is not a lapse.** A sign-in
   link, a registration confirmation or a password reset goes to whatever
   address the person typed; you do not own those domains and cannot enumerate
   them. The built-in `send-magic-link` function ships with `*` for exactly this
   reason, so **sign-up and passwordless sign-in work out of the box** — the
   policy only constrains functions you write yourself.

2. **Resolution.** `enabled` on the config node, then the provider rules above.
   Absent `enabled`, or anything other than `true`, reads as off. That is what
   stops a tenant sending the moment the node appears, but it is also the most
   common cause of "I configured it and nothing happens".

3. **The credential.** Resolving the selected entry's `credential_ref` runs the
   function's *secret* policy too, so a function may hold `email_policy` and
   still be refused the key. The console warns when an enabled config names a
   secret that does not exist on the branch, but it cannot check the key is
   *valid* — nothing reads a secret's value back.

   That is what **Send test** on the Email page is for: the only signal that is
   not indirect. It invokes the built-in `send-test-email` function, which calls
   `raisin.email.send` like any other function — so a green result proves the
   whole chain, not a shortcut around it.

## Recipient caps

At most 20 recipients per send, counted across `to`, `cc` and `bcc`
**together** — not 20 each. Transactional mail is one-to-one or
one-to-a-handful; an unbounded list is an amplification primitive, and anything
larger belongs to a campaign tool.

`cc` and `bcc` take the same shape as `to` (one address or an array) and are
gated by the same `email_policy`. A blind copy is not a way around the
allowlist: every address in all three fields must be allowed, or the whole
message is refused.

A `bcc` address never appears in the message that goes on the wire — only in
the envelope — so no recipient can see who else was blind-copied.

## Attachments

Each entry in `attachments` names exactly **one** source:

```js
await raisin.email.send({
  to: order.email,
  subject: `Ticket ${order.ref}`,
  text: "Your ticket is attached.",
  html: '<p>Your ticket is attached.</p><img src="cid:logo">',
  attachments: [
    // 1. bytes the function already has (base64, or a data: URL)
    { content: pdfBase64, filename: `ticket-${order.ref}.pdf` },

    // 2. a file stored on a node — the server fetches it
    { node: "/tickets/4711", workspace: "assets", property: "file" },

    // 3. a Resource, converted to (2) for you — QuickJS only
    ticketNode.getResource("file"),

    // 4. an inline image, referenced from the html above
    { node: "/brand/logo", workspace: "assets", property: "file",
      contentId: "logo" },
  ],
});
```

`contentType` is optional — it is derived from the filename, or from the stored
resource. `filename` is required for `content` and optional for a node
reference (the stored name is used).

**Inline images.** Set `contentId` and reference it as `<img src="cid:that-id">`.
It requires an `html` body. It works over `smtp` and `resend`; **`brevo` has no
Content-ID at all**, so an inline attachment sent through it is *refused*
before anything is sent, rather than delivered as a stray attachment with a
`cid:` reference pointing at nothing.

**Limits.** By default at most 20 attachments, 10 MiB each and 10 MiB in total
once decoded. Raise them per sender:

```yaml
providers:
  - name: transactional
    provider: resend
    attachments:
      max_total_bytes: 26214400   # 25 MiB
```

Raise `max_total_bytes` and you should expect to raise the send timeout with
it: the 30 s per-send timeout is shared with the upload, and 25 MiB in 30 s
needs a sustained ~7 Mbit/s. A cap that is too high converts a clean rejection
into a timeout half way through sending.

**Where the bytes come from.** A node reference is read through the ordinary
node API, so row-level security applies — but with the **function's** authority,
not the caller's. A function running from a trigger or a schedule reads as
system, so it can attach a file the person who triggered it could not see.
That is already true of `raisin.nodes.get`; it matters more here because the
same call mails the result somewhere. Two things bound it: recipients are
checked against `email_policy` *before* any file is read, and only a resource
property can be attached — this is "attach a file", not "read any property as
one".

There is deliberately **no** way to attach a URL, although both Resend and
Brevo accept one. The provider would fetch it from its own network, outside the
egress policy every other outbound call goes through, and we would never see
the bytes we promise to bound. Fetch it yourself and attach the result.

**Generating the file.** RaisinDB does not render PDFs; `raisin.pdf.*` only
reads them. Either store the file as an asset node first, or call a rendering
service with `fetch` and attach the response bytes.

## Provider notes

- **Brevo has two different credentials, both called a key.** The REST
  transactional API (`provider: brevo`) needs a **v3 API key** from
  *Settings → API keys*. The **SMTP key** from *Settings → SMTP & API → SMTP* is
  a different credential for Brevo's relay: it returns **401** against the REST
  API, with nothing in the error naming the mix-up. To use an SMTP key,
  configure `provider: smtp` against `smtp-relay.brevo.com` instead. This is the
  single most common reason a correct-looking Brevo config never delivers.
- **Brevo requires `htmlContent`.** A text-only message is rejected, so the
  sender derives an escaped HTML part when only `text` is given.
- **Verify the domain first.** Every provider rejects a `from_address` on an
  unverified domain, and the error names the address rather than the domain,
  which reads like a typo.
- **SMTP is served natively, not from the sandbox.** A function never holds the
  socket, the host or the password: it names a provider, and the server dials
  the operator-configured relay. The host goes through the same egress guard as
  every other outbound call, so a loopback or private-network relay is refused
  unless the operator has enabled private egress server-wide. `starttls`
  *requires* the upgrade rather than attempting it, so a relay that lost TLS
  support fails instead of sending the password in the clear.
