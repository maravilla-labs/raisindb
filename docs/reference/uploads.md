# Uploads

Resumable, chunked uploads. A session is opened with `POST /api/uploads`, filled with
`PATCH /api/uploads/{id}` (one `Content-Range` chunk per request), and finished with
`POST /api/uploads/{id}/complete`, which enqueues the job that writes the node.

A signed-in caller needs `create` on the destination (`update` when a node is already
there); that is checked when the session opens and again at completion.

## Upload tokens for devices without a user

A phone that scanned a QR code has no account. An upload token lets a signed-in user
authorize ONE destination for it, without a service identity, a create grant or a
quarantine folder.

Flow:

1. The desktop user (signed in, allowed to create at the destination) mints a token.
2. The token travels to the phone, for example in the QR URL **fragment** (`#t=...`;
   a fragment is never sent to a server, so it stays out of access logs).
3. The phone opens a session with `Authorization: Upload <token>`, sends the chunks,
   and completes. Nothing else on the server accepts the token.

### Mint: `POST /api/uploads/tokens`

Normal user authentication (`Authorization: Bearer ...`). Anonymous callers and upload
tokens are refused (401). The caller's `create` permission on the destination, and on
any folder the upload would have to create, is checked now with the same rules as a
direct upload (403 otherwise).

```json
{
  "repository": "studio",
  "branch": "main",
  "workspace": "assets",
  "folder": "/inbound/phone",
  "extension": "jpg",
  "max_bytes": 20971520,
  "allowed_mimes": ["image/*"],
  "ttl_seconds": 600,
  "uses": 1
}
```

Give exactly one of:

- `path`: the exact destination, for example `/documents/2026/scan.pdf`. One use.
- `folder`: a folder; each use is stored as `<folder>/up-<id>-<n>[.<extension>]`, a name
  the server chooses. `uses` (default 1, max 20) is how many files the token may add.

| field | default | limit |
| --- | --- | --- |
| `branch` | `main` | |
| `node_type` | `raisin:Asset` | |
| `max_bytes` | 50 MiB | 1 GiB |
| `allowed_mimes` | any | up to 20; `image/*` wildcards allowed |
| `ttl_seconds` | 600 | 1 to 3600 |
| `uses` | 1 | 1 to 20 (folder mode) |

Response `201`:

```json
{
  "token": "rut1.<payload>.<signature>",
  "expires_at": "2026-10-20T09:10:00Z",
  "upload_url": "/api/uploads",
  "max_bytes": 20971520,
  "uses": 1
}
```

### Upload with the token

Every call carries `Authorization: Upload <token>` and no other credential.

1. `POST /api/uploads`, body `{"file_size": 1234567, "content_type": "image/jpeg"}`
   (`chunk_size` optional). The repository, branch, workspace, path and file name come
   from the token. A body that names a repository, branch, workspace or path outside the
   token is refused (`403 TOKEN_SCOPE`); other metadata in the body is ignored. A file
   over `max_bytes` is `413 TOKEN_TOO_LARGE`, a content type outside `allowed_mimes`
   (or none, when a list is set) is `415 TOKEN_MIME`. This call takes the token's one use;
   a refused request does not.
   Response `201`: `upload_id`, `upload_url` (`/api/uploads/{id}`), `chunk_size`,
   `total_chunks`, `expires_at`.
2. `PATCH /api/uploads/{id}` with `Content-Range: bytes <start>-<end>/<file_size>` and
   the chunk as the body, repeated until done. `HEAD` and `GET` on the same URL report
   progress.
3. `POST /api/uploads/{id}/complete` with `{}`. Returns `{upload_id, job_id, status}`.
   The write runs as the system, as for any upload; the authorization happened at mint.

`DELETE /api/uploads/{id}` cancels. A token reaches only the sessions it opened; any other
id answers 404.

### Security properties

- The token is an HMAC-SHA256 signature over the claim (tenant, repository, branch,
  workspace, destination, limits, expiry, id, uses). The key is derived per tenant from the
  signing secret (`RAISINDB_SIGNING_SECRET`), with a domain separator of its own, so a
  token for one tenant fails on another and an asset grant is never an upload token.
  Compared in constant time.
- Rotation: while rotating the signing secret, set the old value as
  `RAISINDB_SIGNING_SECRET_PREVIOUS`; tokens signed with it keep verifying until they
  expire (at most one hour). Remove it afterwards.
- The token carries no user and no permission. It is read by the upload endpoints only;
  `Authorization: Upload ...` is not a credential anywhere else, so on every other route
  the request is an unauthenticated one, and `Bearer <token>` is not a valid login.
- Expiry gates opening a session. A session opened in time stays usable for up to one hour,
  so a slow upload can finish after the token's own expiry; it is still limited to that one
  session.
- Size is enforced twice: at session creation, and while a chunk streams (a chunk larger
  than its `Content-Range`, or a range past the declared size, is refused for every caller).
- The commit is attributed to `upload-token`; a token cannot choose the commit actor or
  message.
- Neither the token nor its signature is logged (only the token id).
- A token is create-only. Opening a session and completing it both refuse (`409
  TOKEN_DESTINATION_TAKEN`) when a node already exists at the destination, so a node that
  appeared after the token was minted is never overwritten, and a use that has completed
  cannot be replayed. A session of the same token already heading to that destination also
  blocks a second one.
- The use counter is held in the server process, like the upload sessions themselves. After a
  restart, or on another cluster node, it starts over. The remaining window is exactly this:
  a replay of a use that has NOT yet completed can open one more session for the same fixed
  destination, within the token's limits; whichever completes second is refused because the
  destination then exists.

Browsers: the Studio phone page reads the token from the URL fragment and sends it in the
`Authorization` header; a cross-origin page needs the usual CORS allowance for
`Authorization`, `Content-Range` and `PATCH`.

### Try it

`scripts/upload-token-e2e.mjs` mints, uploads and completes against a running server.
