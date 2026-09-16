# Asset access: two audiences, two instruments

Reading an asset's bytes over HTTP requires a credential in the URL, because the
consumer is often an `<img>` tag, which can carry no header. There are two forms
of that credential, and which one is right depends on WHO is reading.

| | Per-asset signature | Scoped grant |
|---|---|---|
| URL | `…/raisin:display?sig=…&exp=…` | `…/raisin:display?grant=…` |
| Covers | exactly one asset, one property, one command | every asset under a path prefix |
| Authority | the signature IS the authority; the read is unfiltered | none of its own; the read runs as the named subject under row-level security |
| Minted by | `POST …/{path}/raisin:sign` | `POST …/{prefix}/raisin:grant` |
| Audience | one machine fetching one object | an interactive session rendering a page |

The rule is by AUDIENCE, not by convenience. A media service that fetches one
file wants a narrow, disposable capability it can be handed safely. A page
showing sixty thumbnails does not want sixty signatures on sixty independent
clocks — a tab left open long enough starts serving broken images.

## The per-asset signature

Unchanged, and still the default. `POST` to an asset path with `raisin:sign`:

```http
POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:sign
{ "command": "display", "expires_in": 300 }
```

```json
{ "url": "/api/repository/media/main/head/assets/a.jpg/raisin:display?sig=…&exp=…",
  "expires_at": "2026-01-01T00:05:00+00:00" }
```

The signing grammar lives in `raisin_core::asset_urls`, shared by every minter
and by the verifier, because a minter and a verifier that disagree by one byte
produce a URL that always answers 401 with nothing to read.

## The scoped grant

```http
POST /api/repository/{repo}/{branch}/head/{ws}/{prefix}/raisin:grant
{ "expires_in": 900 }
```

```json
{ "grant": "rag1.…",
  "prefix": "/photos",
  "expires_at": "2026-01-01T00:15:00+00:00",
  "expires": 1767225300 }
```

The body is optional; with none, the grant gets the default lifetime. The prefix
is whatever path precedes `raisin:grant`, so `…/head/{ws}/raisin:grant` covers
the whole workspace. The token is then appended to any asset URL in scope:

```
/api/repository/media/main/head/assets/photos/a.jpg/raisin:display?grant=rag1.…
/api/repository/media/main/head/assets/photos/2024/b.png/raisin:display?grant=rag1.…
```

A grant covers every property and both commands (`display` and `download`)
within its prefix. The command only selects `Content-Disposition`; it is not an
access boundary.

### What a grant is, and is not

A grant signs a scope: `(tenant, repo, branch, workspace, path prefix, subject,
expiry)`. It is **not** a wider signature. It confers no authority of its own —
it names WHICH subject and WHICH subtree, and the read is then performed as that
subject, under that subject's row-level security, resolved at the moment of the
read.

That single decision carries the security argument:

* **A grant cannot exceed its subject.** It returns exactly what a direct read by
  the same subject would return — not by assumption, but because it is the same
  read. Minting one over a subtree containing nodes the subject may not read is
  harmless; those nodes stay unreadable, and report as missing rather than as
  forbidden.
* **A grant cannot name anyone else.** The subject, and the `email` / `home`
  claims carried alongside it, are copied from the minting principal's own
  authenticated context, never from the request body.
* **A grant cannot cross a boundary.** Tenant, repo, branch and workspace are all
  in the signed payload and all checked on every read.

Two principals may not mint one, both deliberately:

* **Anonymous** — a grant bound to nobody is a plain bearer token for whatever
  anonymous may read, which public asset delivery covers better.
* **System / admin** — their context bypasses row-level security, so a grant
  naming them would be a standing key to a subtree with no filtering behind it.
  They already have per-asset signing, which is the right instrument for them.

### Client contract

Mint once per page load, append to every asset URL in scope, and **renew on 401,
once**. A stale grant then costs one round trip and a retry rather than a broken
page; that, not a longer expiry, is what removes the expiry problem.

The response codes are the retry contract:

| Status | Code | Meaning |
|---|---|---|
| 401 | `CREDENTIAL_EXPIRED` | the grant's clock ran out — renew |
| 401 | `INVALID_GRANT` | not a grant this deployment minted — renew once, then give up |
| 403 | `GRANT_OUT_OF_SCOPE` | a valid grant for a different subtree — renewing the same grant will not help |
| 404 | — | no such node, or none the subject may read (the two are deliberately indistinguishable) |

A request presenting no grant keeps the answer it has always had —
`401 INVALID_SIGNATURE` — whether its signature was absent, wrong or expired.
The finer codes are reserved for grants, because only a grant client acts on the
difference.

## The four questions the design left open

**1. Can one verifier accept both forms?** Yes, and it must.
`raisin_core::authorize_asset_read` is the single entry point; the serve handler
has no second branch that checks a credential. A verifier that forks drifts, and
a drifted verifier is a URL that 401s with nothing to say why, since the
signature is a hash and there is no diff to read. What the two forms *do* differ
in is where the node is read from, and that difference is the feature: a
signature reads unfiltered, a grant reads as its subject.

The two also share a signing secret, so the grant HMAC is domain-separated. A
grant's signature can never verify as an asset URL's, or the reverse.

**2. Workspace-scoped, or an arbitrary prefix?** Arbitrary prefix, matched on
path SEGMENTS. A string prefix says `/photos` contains `/photos-private`,
because the characters line up. `prefix_covers` normalizes both sides and
compares with the boundary explicit, so `/photos` covers `/photos` and
`/photos/…` and nothing else. A path or prefix containing `.` or `..` is refused
outright rather than resolved: traversal input on this surface is malformed, and
quietly resolving it would give one path two spellings, which is exactly the
condition segment matching exists to rule out.

**3. Revocation.** Delegated to the permission system rather than to the token,
and this is why the read is performed as the subject. Permissions are resolved on
the read path, so withdrawing a subject's access closes every grant naming them —
no counter, no revocation list, and no new lookup that was not already on a
session's read path. Two bounds apply in practice:

* the permission cache's TTL, which governs live sessions identically, and
* the grant's own expiry: **15 minutes by default, one hour maximum**
  (`DEFAULT_GRANT_LIFETIME_SECS`, `MAX_GRANT_LIFETIME_SECS`).

The expiry is the outer bound on a stolen token, not the revocation mechanism.

**4. Query parameter or cookie?** Query parameter. A cookie would stay out of
logs and referrers, but it would have to be scoped to a path on the serving
origin — which fails as soon as assets are served from a configured base URL
different from the app's — and it would then be sent on every request to that
origin, which is ambient authority of exactly the kind this design avoids. The
query parameter is also the only thing an `<img>`, `<video>` or CSS `url()` can
carry.

The leak risk that decision accepts is answered by the grant's shape rather than
by hiding it: a leaked grant is useless for anything its subject cannot read, it
stops working when that subject's access is withdrawn, and it expires in
minutes. A per-asset signature already rides in the URL on the same terms.

## What is NOT replaced

Per-asset signing, for anything server-to-server. Handing a subtree-wide token to
an out-of-process media service would be a standing key to that subtree, which is
strictly worse for that job than the one URL it needs.

A grant is also not public delivery. A published site serving public assets to
anonymous readers should serve them publicly; the grant is for editing and
preview surfaces, where a real session exists and row-level security is what
decides.

## Related, and shipped separately

Byte-range support in the serve path (`Accept-Ranges`, `206`, `Content-Range`).
Without it a browser cannot seek in a video or audio file at all. It applies to
both credential forms.

## Where the code is

* `crates/raisin-core/src/asset_grants.rs` — the grant, the token, segment
  matching, and `authorize_asset_read`, the one verifier.
* `crates/raisin-core/src/asset_urls.rs` — the per-asset signing grammar.
* `crates/raisin-transport-http/src/handlers/repo/assets.rs` — minting both
  forms, and the serve path that reads unfiltered for a signature and as the
  subject for a grant.
* `crates/raisin-transport-http/tests/all/http_asset_grants.rs` — the refusals
  and the permitted read, end to end.
