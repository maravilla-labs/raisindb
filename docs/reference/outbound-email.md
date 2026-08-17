# Outbound email

Transactional mail — magic-link sign-in, notifications, receipts. Each tenant
sends through **their own** provider account from **their own** verified domain.

There is deliberately no Maravilla fallback sender. A tenant that has configured
nothing cannot send at all, rather than sending as somebody else.

## The two halves

Configuration is split, and the split is the point: the settings are a node (so
they are versioned, auditable and diffable), while the credential is not (so it
is never in a node property, a revision, or a diff).

| | Where | Set via |
|---|---|---|
| Settings | `raisin:EmailConfig` node at `/config/email` in the `raisin:system` workspace | **Email** page in the admin console |
| API key | secret store, named by `credential_ref` (default `email/api_key`) | **Secrets** page |

Both are per repository **branch** — the node lives in a branch's workspace and
the secret store is keyed `{tenant, repo, branch}`. A branch forked after the
config exists inherits both.

## Fields

| Field | Notes |
|---|---|
| `enabled` | Master switch, **defaults to false** |
| `provider` | `resend` or `brevo` |
| `from_address` | Must be on a domain verified with *that tenant's* provider account |
| `from_name`, `reply_to` | Optional |
| `base_url` | Absolute URL of the tenant's front end — magic links are built from it |
| `credential_ref` | `secret://email/api_key`; a reference, never a key |

HTTP providers only. The function sandbox has no raw TCP, so SMTP is absent by
construction rather than by omission — adding it is a schema change plus a
transport, not a config option.

## Sending

One `ApiMethodDescriptor` serves both runtimes, so QuickJS and Starlark see the
same call and the same behaviour:

```js
// QuickJS
raisin.email.send({ to: ["a@example.com"], subject: "Hi", text: "..." })
```

```python
# Starlark
raisin.email.send({"to": ["a@example.com"], "subject": "Hi", "text": "..."})
```

Only the message is an argument. The sender identity and the credential come
from the config node, never from the caller — a function cannot choose who it
appears to be.

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

   This runs *first* — before the config is read and before the credential is
   decrypted — so a function that was never granted email learns it from the
   policy rather than from a provider error, and a denied send never causes the
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

   A domain list is the right shape for mail with a known audience: staff
   notifications, internal alerts, or a deployment where only your own people
   may sign in. Narrowing the magic-link function is the single edit that turns
   it into a closed-audience mailer, which is a deliberate choice rather than a
   default.

2. **`enabled` on the config node.** Absent or anything other than `true` reads
   as off. That is what stops a tenant sending the moment the node appears, but
   it is also the most common cause of "I configured it and nothing happens".

3. **The credential.** Resolving `credential_ref` runs the function's *secret*
   policy too, so a function may hold `email_policy` and still be refused the
   key. The console warns when an enabled config names a secret that does not
   exist on the branch, but it cannot check the key is *valid* — nothing reads a
   secret's value back, so the first proof is a real send.

## Recipient caps

At most 20 recipients per send. Transactional mail is one-to-one or
one-to-a-handful; an unbounded list is an amplification primitive, and anything
larger belongs to a campaign tool.

## Provider notes

- **Brevo requires `htmlContent`.** A text-only message is rejected, so the
  sender derives an escaped HTML part when only `text` is given.
- **Verify the domain first.** Both providers reject a `from_address` on an
  unverified domain, and the error names the address rather than the domain,
  which reads like a typo.
