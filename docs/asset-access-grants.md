# Asset access: two audiences, two instruments

## The problem

Every asset read is authorised by a signature minted for ONE asset, with an
expiry: `…/raisin:display?sig=…&exp=…`. That is right for one audience and
wrong for the other, and today both use it.

**Machine-to-machine — keep it exactly as it is.** The media pipeline hands a
URL to an out-of-process service that fetches it once. A narrow, short-lived,
disposable capability for a single object is precisely what that wants. It
carries no ambient authority and expires before it can be reused.

**A browser — this is where it breaks down.** Studio's asset grid renders 60
thumbnails, which means 60 signatures, each expiring on its own clock. A tab
left open for an hour starts serving broken images. Every new surface that
displays an asset has to remember to sign first, and the signature is
re-establishing authorisation the SESSION already established: the user is
logged in, and RLS already decides what they may read.

## Proposed: a scoped grant, not a broader signature

Sign a SCOPE once — `(tenant, repo, branch, workspace, path prefix, user id,
expiry)` — and let every asset under that prefix validate against the same
token. The client appends one `?grant=…` to any asset URL in scope.

Three properties are what make widening the scope acceptable:

1. **Bound to the user id.** A grant is not a transferable bearer capability;
   a leaked one is useless to anyone else. This is the main objection to
   widening scope, and binding answers it directly.
2. **Derived from RLS, never from the request.** The grant says "what this user
   can already read here". It therefore cannot exceed what a direct read would
   return, which makes "they are logged in anyway" true BY CONSTRUCTION rather
   than by assumption. A grant whose scope came from the client would be an
   escalation primitive.
3. **Renew on 401, once.** A stale grant costs one round trip and a retry, not
   a broken page. That is what actually removes the expiry problem — a longer
   expiry only moves it.

## What this does NOT replace

Per-asset signing stays for anything server-to-server. The narrow capability is
the point there: minimal, disposable, and safe to hand to another process. A
grant handed to a media service would be a standing key to a subtree, which is
strictly worse for that job.

So the rule is by AUDIENCE, not by convenience: interactive session → grant;
one machine fetching one object → signature.

## Open questions, to settle before building

- **Can `serve_asset` verify both without becoming two code paths?** If the
  verifier forks, the two drift, and a drifted verifier is a URL that 401s with
  nothing to say why. One verification entry point that accepts either form.
- **Workspace-scoped or arbitrary path prefix?** Prefix is more flexible and
  considerably easier to get subtly wrong — a prefix match that does not respect
  path segment boundaries lets `/photos-private` fall inside a grant for
  `/photos`. If prefixes are allowed, match on segments, never on strings.
- **Revocation.** A short expiry is the only revocation a stateless grant has.
  If that is not enough — a user losing access mid-session — the grant needs a
  generation counter to check against, which reintroduces a lookup on the read
  path. Decide deliberately which is wanted.
- **Does it belong in the URL at all?** A cookie scoped to the asset path would
  not appear in logs, referrers or shared links. A query parameter is easier to
  use from an `<img>` and easier to leak.

## Related, and shipped separately

Byte-range support in `serve_asset` (`Accept-Ranges`, `206`, `Content-Range`).
Without it a browser cannot seek in a video or audio file at all — it can only
take the whole body or nothing. Delivery already solved this in
`delivery/src/utils/http_range.rs`; raisindb had no range handling whatsoever.
