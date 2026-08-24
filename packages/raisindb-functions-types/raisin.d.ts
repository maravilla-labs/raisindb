/**
 * RaisinDB Server-Side Function Runtime Type Definitions
 *
 * These types describe the `raisin` global object available inside
 * RaisinDB server-side functions (QuickJS runtime).
 *
 * This is NOT Node.js — no `Buffer`, `fs`, `require()`, or npm modules.
 *
 * Available globals (beyond the raisin.* API):
 * - ES module imports with relative paths: import { foo } from './utils.js'
 * - W3C Fetch API: fetch(), Request, Response, Headers, ReadableStream, AbortController, FormData
 * - Timers: setTimeout, clearTimeout, setInterval, clearInterval
 * - Console: console.log, console.debug, console.warn, console.error
 *
 * Auto-generated from: crates/raisin-functions/src/runtime/bindings/
 */

// ==========================================================================
// Resource & Node Types (from api_wrapper.js — not in the bindings registry)
// ==========================================================================

/**
 * A binary file resource attached to a node property.
 * Returned by `node.getResource('./file')`.
 *
 * Provides built-in server-side image resizing and PDF processing.
 * This is NOT Node.js — no Buffer, fs, require(), or npm modules.
 * fetch() IS available (W3C Fetch API). ES module imports with relative paths are supported.
 * Use resource.resize() for images and resource.processDocument() for PDFs.
 */
declare class Resource {
  /** Unique identifier */
  readonly uuid: string;
  /** Original filename */
  readonly name: string;
  /** File size in bytes */
  readonly size: number;
  /** MIME type (e.g., "image/jpeg", "application/pdf") */
  readonly mimeType: string;
  /** Storage metadata */
  readonly metadata: Record<string, any>;
  /** Internal storage key */
  readonly storageKey: string | null;

  /** Get binary data as base64 string. */
  getBinary(): Promise<string>;

  /** Get as data URL (data:mime;base64,...). */
  toDataUrl(): Promise<string>;

  /**
   * Resize image server-side. Returns a NEW Resource with the resized data.
   * This is the ONLY way to create thumbnails. Do NOT use sharp, jimp, Canvas,
   * or any external library — they do not exist in this runtime.
   *
   * @example
   * const resource = node.getResource('./file');
   * const thumbnail = await resource.resize({ maxWidth: 200, format: 'jpeg', quality: 80 });
   * await node.addResource('./thumbnail', thumbnail);
   */
  resize(options: ResizeOptions): Promise<Resource>;

  /**
   * Convert a PDF page to an image. Returns a new Resource.
   * Only works with PDF files (mimeType contains "pdf").
   */
  toImage(options?: PdfToImageOptions): Promise<Resource>;

  /** Get page count for PDF files. Only works with PDFs. */
  getPageCount(): Promise<number>;

  /**
   * Process PDF document server-side: extract text, OCR, generate thumbnail.
   * Uses storage-key-based API (no base64 overhead). Only works with PDFs.
   *
   * @example
   * const resource = node.getResource('./file');
   * const result = await resource.processDocument({ generateThumbnail: true, thumbnailWidth: 200 });
   * if (result.thumbnail) {
   *   await node.addResource('./thumbnail', result.thumbnail);
   * }
   */
  processDocument(options?: ProcessDocumentOptions): Promise<DocumentResult>;
}

interface ResizeOptions {
  /** Maximum width in pixels */
  maxWidth?: number;
  /** Maximum height in pixels */
  maxHeight?: number;
  /** Output format */
  format?: 'jpeg' | 'png' | 'webp';
  /** Quality 1-100 (JPEG/WebP only) */
  quality?: number;
}

interface PdfToImageOptions {
  /** Page number (0-indexed, default 0) */
  page?: number;
  /** Maximum width in pixels */
  maxWidth?: number;
  /** Output format (default 'jpeg') */
  format?: 'jpeg' | 'png' | 'webp';
  /** Quality 1-100 */
  quality?: number;
}

interface ProcessDocumentOptions {
  /** Enable OCR for scanned PDFs */
  ocr?: boolean;
  /** OCR languages (default ["eng"]) */
  ocrLanguages?: string[];
  /** Generate a thumbnail of the first page */
  generateThumbnail?: boolean;
  /** Thumbnail width in pixels */
  thumbnailWidth?: number;
}

interface DocumentResult {
  /** Extracted text content */
  text: string;
  /** Number of pages */
  pageCount: number;
  /** Whether the PDF appears to be scanned */
  isScanned: boolean;
  /** Whether OCR was used */
  ocrUsed: boolean;
  /** Extraction method used */
  extractionMethod: string;
  /** Thumbnail Resource (if generateThumbnail was true) */
  thumbnail?: Resource;
}

/**
 * A node returned by raisin.nodes.get() and similar methods.
 * Includes helper methods for binary resource operations.
 */
interface RaisinNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  archetype?: string;
  properties: Record<string, any>;
  created_at?: string;
  updated_at?: string;

  /**
   * Get a Resource handle for a binary property.
   * @param propertyPath - e.g., "./file" or "file"
   * @returns Resource with resize(), processDocument(), etc., or null if not present
   */
  getResource(propertyPath: string): Resource | null;

  /**
   * Upload/store a Resource on a node property.
   * @param propertyPath - Target property, e.g., "./thumbnail"
   * @param data - Resource (from resize()), or { base64, mimeType, name }
   */
  addResource(propertyPath: string, data: Resource | ResourceUploadData | string): Promise<any>;

  /**
   * True if this node carries the given mixin (type-declared, transitively).
   * Backed by the server-materialized effective-mixin set.
   * @param mixinName - e.g., "app:SEO"
   */
  hasMixin(mixinName: string): boolean;

  /**
   * True if this node "is a" given type — its node_type, any `extends` ancestor,
   * or any effective mixin.
   * @param typeName - e.g., "app:Folder"
   */
  isNodeType(typeName: string): boolean;
}

interface ResourceUploadData {
  base64: string;
  mimeType: string;
  name?: string;
}

/**
 * A single entry in a node's revision history (newest first), returned by
 * raisin.nodes.history(). Use `revision` with an at-revision read to fetch the
 * full snapshot of that historical version.
 */
interface RaisinRevisionEntry {
  /** HLC revision string ("timestamp-counter"). */
  revision: string;
  /** When this revision was written (ISO 8601 string). */
  updated_at?: string;
  /** User ID who authored this revision. */
  updated_by?: string;
  /** True when the node was deleted at this revision (tombstone). */
  deleted: boolean;
}

interface NodeCreateData {
  name?: string;
  path?: string;
  node_type: string;
  properties?: Record<string, any>;
}

/** Execution context available as raisin.context */
interface ExecutionContext {
  tenant_id: string;
  repo_id: string;
  branch: string;
  workspace_id: string;
  actor?: string;
  execution_id?: string;
}

/** Context passed to every function handler */
interface FunctionContext {
  flow_input: {
    event: {
      node_id: string;
      node_type: string;
      node_path: string;
      event_type: string;
    };
    workspace: string;
  };
}

interface NotifyOptions {
  title: string;
  body?: string;
  recipient?: string;
  recipientId?: string;
  priority?: 'low' | 'normal' | 'high';
  type?: string;
  link?: string;
  data?: Record<string, any>;
}

interface HttpOptions {
  method?: string;
  headers?: Record<string, string>;
  body?: any;
  params?: Record<string, string>;
  timeout?: number;
}

interface HttpResponse {
  status: number;
  headers: Record<string, string>;
  body: any;
}

interface AiCompletionRequest {
  model: string;
  messages: Array<{ role: 'system' | 'user' | 'assistant'; content: string }>;
  response_format?: { type: 'json_object'; schema?: any };
  temperature?: number;
  max_tokens?: number;
}

interface AiEmbedRequest {
  model: string;
  input: string | string[];
  input_type?: 'search_document' | 'search_query';
}

// ==========================================================================
// The raisin Global Object (auto-generated from bindings registry)
// ==========================================================================

declare namespace raisin {
  namespace ai {
    function completion(request: any): Promise<any>;
    function listModels(): Promise<any[]>;
    function getDefaultModel(useCase: string): Promise<any | null>;
    function embed(request: any): Promise<any>;
  }

  namespace branches {
    function diff(branch: string, baseBranch: string): Promise<any>;
    function compare(branch: string, baseBranch: string): Promise<any>;
    function copyNodes(sourceBranch: string, targetBranch: string, opts: any): Promise<any>;
  }

  namespace crypto {
    function uuid(): Promise<string>;
    function verifyJwt(token: string, opts?: any | null): Promise<any>;
    /** n cryptographically secure random bytes, base64-encoded. n: 1..=64. */
    function randomBytes(n: number): Promise<string>;
    /** Lowercase hex digest. alg: "sha256" (default) | "sha512". */
    function hash(input: string, alg?: string | null): Promise<string>;
    /** Generate a signing keypair; alg defaults to "ES256" (ECDSA P-256). */
    function generateKeyPair(
      alg?: string | null
    ): Promise<{ alg: string; publicJwk: any; privateJwk: any }>;
    /** Sign claims into a compact JWS. Signature is JOSE r||s base64url. */
    function signJwt(
      claims: any,
      privateJwk: any,
      opts?: { alg?: string; kid?: string; expiresInSec?: number } | null
    ): Promise<string>;
  }

  namespace date {
    function now(): Promise<string>;
    function timestamp(): Promise<number>;
    function timestampMillis(): Promise<number>;
    function parse(dateStr: string, format?: string | null): Promise<number>;
    function format(timestamp: number, format?: string | null): Promise<string>;
    function addDays(timestamp: number, days: number): Promise<number>;
    function diffDays(ts1: number, ts2: number): Promise<number>;
  }

  /**
   * One outbound transactional email. The sender is deliberately absent:
   * `from`, the display name and `replyTo` come from the tenant's
   * `/config/email` node, so a function cannot send as an unverified address.
   */
  interface EmailMessage {
    /** One recipient address, or several. */
    to: string | string[];
    /** Carbon copy. Visible to every recipient. */
    cc?: string | string[];
    /**
     * Blind carbon copy. Each address is invisible to every other recipient.
     *
     * Counted against the same 20-recipient cap as `to` and `cc`, and checked
     * against the function's `email_policy` exactly like them — a blind copy
     * is not a way around the allowlist.
     */
    bcc?: string | string[];
    subject: string;
    /** Plain-text body. Always required, even alongside `html`. */
    text: string;
    html?: string;
    /**
     * Files to attach. Each entry names exactly one source.
     *
     * Defaults: at most 20 attachments, 10 MiB each and 10 MiB in total once
     * decoded. An operator can raise them per sender in `/config/email`.
     */
    attachments?: EmailAttachment[];
    /**
     * Which of the tenant's configured senders to send through, by name (see
     * {@link email.providers}). Omit it for the tenant's default — which is
     * what system mail such as the magic link uses.
     *
     * An unknown name REJECTS rather than falling back to the default: mail
     * leaving through the wrong account is worse than mail not leaving.
     */
    provider?: string;
  }

  /** Fields every attachment shares, whatever its source. */
  interface EmailAttachmentBase {
    /**
     * Name the recipient sees. Required for `content`; for a node reference
     * the stored file name is used when omitted.
     *
     * Rejected rather than cleaned up if it carries a path separator or a
     * control character.
     */
    filename?: string;
    /**
     * MIME type. Derived from the filename (or the stored resource) when
     * omitted. `multipart/*` and `message/*` are refused — neither can be a
     * single attachment.
     */
    contentType?: string;
    /**
     * Set this to EMBED the file in the HTML body instead of listing it as a
     * download: reference it as `<img src="cid:the-value">`.
     *
     * Requires an `html` body. Works over `smtp` and `resend`; `brevo` has no
     * Content-ID and REJECTS an inline attachment rather than sending one that
     * would arrive broken.
     */
    contentId?: string;
  }

  /** Bytes the function already holds. */
  interface EmailAttachmentContent extends EmailAttachmentBase {
    /** Standard base64, or a `data:<type>;base64,...` URL. */
    content: string;
    filename: string;
    node?: never;
  }

  /**
   * A file stored on a node, fetched by the server.
   *
   * Read with the FUNCTION's authority, not the caller's — a function running
   * from a trigger or a schedule reads as system. Treat it exactly as you
   * treat {@link nodes.get}.
   */
  interface EmailAttachmentNode extends EmailAttachmentBase {
    /** Path of the node holding the file. */
    node: string;
    /** Workspace the node lives in. Required. */
    workspace: string;
    /** Property holding the file. Defaults to `"file"`. */
    property?: string;
    content?: never;
  }

  /**
   * One attachment: inline bytes, or a node reference.
   *
   * In JavaScript a `Resource` (from `node.getResource('file')`) may be passed
   * directly and is converted to a node reference for you.
   */
  type EmailAttachment = EmailAttachmentContent | EmailAttachmentNode;

  /** Proof that the provider accepted a message. Acceptance is not delivery. */
  interface EmailReceipt {
    /** The provider's message id — what a later bounce/webhook correlates to. */
    message_id: string;
    /** The provider API that issued it: "resend", "brevo" or "smtp". */
    provider: string;
    /**
     * The configured sender it went through. Answers "which of my accounts
     * sent this", which `provider` alone cannot once a tenant has two entries
     * on the same API.
     */
    sender: string;
  }

  /** One of the tenant's configured senders, as {@link email.providers} lists it. */
  interface EmailProviderInfo {
    /** The name {@link email.send} accepts in `provider`. */
    name: string;
    /** The provider API behind it: "resend", "brevo" or "smtp". */
    provider: string;
    from_address: string;
    /** A disabled sender cannot be selected, by name or as the default. */
    enabled: boolean;
    /** True for the one system mail goes through. */
    default: boolean;
  }

  /** What {@link email.providers} returns. */
  interface EmailProviders {
    /** The tenant master switch — off means no sender works, however many exist. */
    enabled: boolean;
    providers: EmailProviderInfo[];
  }

  namespace email {
    /**
     * Send one transactional email through the tenant's configured provider.
     *
     * Every recipient must be allowed by the function's `email_policy`
     * (`{ enabled, allowed_recipients }` in its `.node.yaml`, matched against
     * the recipient DOMAIN); with no block declared the function cannot send,
     * and one disallowed recipient rejects the whole message. "Every
     * recipient" includes `cc` and `bcc`.
     *
     * Also rejects when email is not configured or not enabled for the tenant,
     * when the function's `secret_policy` does not grant the credential the
     * config references, or when the provider refuses the message.
     */
    function send(message: EmailMessage): Promise<EmailReceipt>;

    /**
     * List the tenant's configured senders, so a function can discover the
     * names `send` accepts rather than hardcoding one it cannot verify.
     *
     * Carries no credential and no `credential_ref`: a function that may send
     * does not thereby get to enumerate the secret store. Gated on the same
     * `email_policy` as {@link send} — a function that may not send has no use
     * for the names.
     */
    function providers(): Promise<EmailProviders>;
  }

  /** A tenant auth identity — what a magic link or password check resolves to. */
  interface Identity {
    id: string;
    email: string;
    /** True only once the magic-link verify step has proven possession. */
    email_verified: boolean;
    display_name: string | null;
    /** Whether the account has local (password) credentials. */
    has_password: boolean;
  }

  interface IdentityPatch {
    /** New address; trimmed and lowercased. Clears `email_verified`. */
    email?: string;
    /** New password. Gives a magic-link-only account local credentials. */
    password?: string;
    display_name?: string;
  }

  /**
   * Tenant identities (the auth records, NOT `raisin:User` nodes).
   *
   * Gated by the function's `identity_policy: { enabled: true }` in its
   * `.node.yaml`; with no block declared every call rejects with
   * `[identities:policy_denied]`.
   */
  namespace identities {
    /** Look up an identity by (case-insensitive) email. `null` when none. */
    function findByEmail(email: string): Promise<Identity | null>;
    /**
     * Change an identity's email, password and/or display name.
     *
     * Rejects with `[identities:email_taken]` when another account already
     * holds the new address, and with `[identities:invalid_patch]` for any
     * other key — `email_verified` in particular cannot be set here: a rename
     * proves typing, not possession. When the email changes, the bound
     * `raisin:User` node's `email` property follows.
     */
    function update(id: string, patch: IdentityPatch): Promise<Identity>;
  }

  namespace events {
    function emit(eventType: string, data: any): Promise<void>;
  }

  namespace flows {
    function run(flowPath: string, input: any): Promise<any>;
  }

  namespace functions {
    function execute(functionPath: string, arguments: any, context: any): Promise<any>;
    function call(functionPath: string, arguments: any): Promise<any>;
  }

  namespace http {
    /** Make an HTTP request. */
    function request(method: string, url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP GET */
    function get(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP POST */
    function post(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP PUT */
    function put(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP PATCH */
    function patch(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP DELETE */
    function delete(url: string, options?: HttpOptions): Promise<HttpResponse>;
  }

  namespace imap {
    function fetchSince(conn: any, sinceUid: number, opts?: any | null): Promise<any>;
    function listMailboxes(conn: any): Promise<any>;
    function fetchMessage(conn: any, uid: number, opts?: any | null): Promise<any>;
  }

  namespace integrations {
    function syncNow(mountId: string, mode?: string | null): Promise<any>;
  }

  namespace inventory {
    function claim(pool: string, n: number, capacity: number): Promise<any>;
    function release(pool: string, n: number): Promise<number>;
  }

  namespace locks {
    function acquire(key: string, ttlMs: number, owner?: string | null): Promise<any>;
    function release(key: string, token: number): Promise<boolean>;
    function renew(key: string, token: number, ttlMs: number): Promise<boolean>;
  }

  namespace nodes {
    function get(workspace: string, path: string): Promise<RaisinNode | null>;
    function getById(workspace: string, id: string): Promise<RaisinNode | null>;
    function history(workspace: string, id: string, limit?: number | null): Promise<RaisinRevisionEntry[]>;
    function create(workspace: string, parentPath: string, data: any): Promise<RaisinNode>;
    function update(workspace: string, path: string, data: any): Promise<RaisinNode>;
    function delete(workspace: string, path: string): Promise<void>;
    function updateProperty(workspace: string, nodePath: string, propertyPath: string, value: any): Promise<void>;
    function move(workspace: string, nodePath: string, newParentPath: string): Promise<RaisinNode>;
    function query(workspace: string, query: any): Promise<RaisinNode[]>;
    function getChildren(workspace: string, parentPath: string, limit?: number | null): Promise<RaisinNode[]>;
    function applyChildOrder(workspace: string, parentPath: string, sourceBranch: string, targetBranch: string): Promise<void>;
    function addResource(workspace: string, nodePath: string, propertyPath: string, uploadData: any): Promise<any>;
    /**
     * Create a node under parentPath, auto-creating any missing ancestor folders.
     */
    function createDeep(workspace: string, parentPath: string, data: any, parentNodeType?: string): Promise<RaisinNode>;
    /**
     * Upsert a node by path (create-or-update), auto-creating any missing ancestor folders.
     */
    function upsertDeep(workspace: string, data: any, parentNodeType?: string): Promise<void>;
    /**
     * Start a transaction for atomic multi-node operations.
     * @example
     * const tx = raisin.nodes.beginTransaction();
     * tx.create(workspace, parentPath, data);
     * tx.commit();
     */
    function beginTransaction(): Transaction;
  }

  namespace pdf {
    function processFromStorage(storageKey: string, options: any): Promise<any>;
  }

  namespace resources {
    function getBinary(storageKey: string): Promise<string>;
  }

  namespace scheduler {
    function schedule(request: any): Promise<any>;
    function cancel(jobIdOrKey: string): Promise<any>;
    function list(filter?: any | null): Promise<any>;
    function get(jobIdOrKey: string): Promise<any>;
  }

  /**
   * Metadata about one secret. Never carries the secret itself — the type has
   * no field that could hold ciphertext or plaintext.
   */
  interface SecretMetadata {
    name: string;
    /** Human-facing ordinal, from 1. Use it as `secrets.get(name, version)`. */
    version: number;
    /** Which master key sealed it. */
    key_id: number;
    /** RFC 3339. */
    created_at: string;
    created_by: string;
    /** Set only by `rotate`. */
    rotated_at?: string | null;
    /** The node this secret backs, when it is a vaulted schema field. */
    owner_node?: string | null;
    owner_field?: string | null;
    /** True when the newest version is a tombstone. */
    deleted: boolean;
    ciphertext_len: number;
  }

  /**
   * Encrypted secret store, scoped to the current `{tenant, repo, branch}`.
   *
   * **Access is denied by default.** A function reaches these only if its
   * `.node.yaml` declares a matching grant:
   *
   * ```yaml
   * secret_policy:
   *   enabled: true
   *   allowed_names:
   *     - "stripe/*"
   *     - "sendgrid_api_key"
   * ```
   *
   * Without one, every call below throws a `policy_denied` error naming the
   * secret. That default is deliberate: adapters run privileged and hold
   * `raisin.http`, so an ungated secrets binding would be a one-line
   * exfiltration path for every credential in the repo.
   *
   * Every method THROWS on failure (denial, missing secret, deleted secret,
   * unconfigured store) — none of them degrade to `null`, which would be
   * indistinguishable from an empty credential.
   *
   * ## The main flow: a secret stored in a node property
   *
   * The common case is not a standalone secret — it is a field on a regular
   * node declared `encrypted: true` in its NodeType. Three steps:
   *
   * 1. The property does not hold the credential. It holds a REFERENCE:
   *    `"secret://node/01H8XY.../api_key@1"`.
   * 2. A node read returns that string verbatim. **Reads never resolve** —
   *    there is no query flag and no endpoint that returns a value.
   * 3. The function passes the string straight to `get` (or `resolve`):
   *
   * ```javascript
   * const node = await raisin.nodes.get('data', '/connections/stripe');
   * const key  = raisin.secrets.get(node.properties.api_key);
   * ```
   *
   * Pass the reference through **as-is**. Do not strip `secret://` or the
   * `@version` suffix yourself: a name may itself contain `@` (an operator
   * name like `ops@example.com`), so only a trailing all-digit run after the
   * LAST `@` is a version. `get` applies that rule using the same parser the
   * storage layer uses.
   */
  /**
   * Declared as an interface rather than a `namespace`, because `delete` is a
   * reserved word that TypeScript rejects as an ambient `function` name but
   * accepts as an interface method. (The `http`, `nodes` and `admin.nodes`
   * namespaces above still use `function delete(...)`, which does not parse —
   * a pre-existing break, unnoticed because this package has no drift test.)
   */
  interface SecretsApi {
    /**
     * Read a secret's plaintext.
     *
     * `nameOrRef` is EITHER a bare name (`"stripe_key"`) or a full reference
     * (`"secret://node/01H8XY.../api_key@1"`) — see the three-step flow above
     * for why the reference form is what you will usually be holding.
     *
     * A version pinned in the reference is HONOURED: `secret://k@1` returns
     * version 1, not the latest. That is what makes reading an older node
     * revision give the value that revision actually held.
     *
     * Passing the `version` argument **and** a pinned reference throws. Two
     * stated versions cannot both be satisfied, and silently preferring either
     * could return a value the node revision never held — which is the exact
     * guarantee a pinned reference exists to provide. Pass one or the other;
     * `get('k', 2)` and `get('secret://k', 2)` are both fine.
     *
     * The policy allow-list is matched against the parsed NAME, so both
     * spellings of one secret always get the same allow/deny answer.
     *
     * Throws if the policy denies the name, or the secret is missing or
     * deleted.
     */
    get(nameOrRef: string, version?: number): Promise<string>;
    /**
     * Resolve a value that MAY be a `secret://` reference.
     *
     * Returns the plaintext when it is one, or the value unchanged when it is
     * not — so a config field that is a literal password on one deployment and
     * a vaulted reference on another is read the same way:
     *
     * ```javascript
     * const password = raisin.secrets.resolve(conn.password);
     * ```
     *
     * A reference that fails to resolve THROWS; it never falls back to
     * returning the reference text, which would send `secret://...` to a
     * provider as a credential. A plain literal is passed through without any
     * policy check, since no secret was touched.
     */
    resolve(value: string): Promise<string>;
    /**
     * Append a new version. Never overwrites — prior versions stay readable.
     *
     * Accepts a bare name or an UNPINNED reference; a pinned one
     * (`secret://k@1`) is refused, because a write appends a new version
     * rather than replacing that one.
     */
    put(name: string, value: string): Promise<{ name: string; version: number }>;
    /**
     * Metadata for the newest version of every secret this function may read.
     * Never returns values, and is filtered to the policy's allowed names.
     */
    list(): Promise<SecretMetadata[]>;
    /**
     * Append a new version stamped as a rotation. Pinned `secret://name@N`
     * references keep resolving to the old version.
     */
    rotate(name: string, value: string): Promise<{ name: string; version: number }>;
    /** Append a tombstone. Prior versions remain readable by pinned reference. */
    delete(name: string): Promise<{ name: string; version: number }>;
  }

  const secrets: SecretsApi;

  namespace sql {
    function query(sql: string, params: any[]): Promise<any>;
    function execute(sql: string, params: any[]): Promise<number>;
  }

  namespace tasks {
    function create(request: any): Promise<any>;
    function update(task_id: string, updates: any): Promise<any>;
    function complete(task_id: string, response: any): Promise<any>;
    function query(query: any): Promise<any[]>;
  }

  // Transaction methods are accessed via raisin.nodes.beginTransaction()
  /** Send a notification to a user. */
  function notify(options: NotifyOptions): Promise<any>;

  /** Admin methods that bypass row-level security. Requires requiresAdmin: true in function metadata. */
  namespace admin {
    namespace nodes {
      function get(workspace: string, path: string): Promise<any | null>;
      function getById(workspace: string, id: string): Promise<any | null>;
      function create(workspace: string, parentPath: string, data: any): Promise<any>;
      function update(workspace: string, path: string, data: any): Promise<any>;
      function delete(workspace: string, path: string): Promise<void>;
      function updateProperty(workspace: string, nodePath: string, propertyPath: string, value: any): Promise<void>;
      function query(workspace: string, query: any): Promise<any[]>;
      function getChildren(workspace: string, parentPath: string, limit?: number | null): Promise<any[]>;
    }
    namespace sql {
      function query(sql: string, params: any[]): Promise<any>;
      function execute(sql: string, params: any[]): Promise<number>;
    }
  }

  /** Execution context with tenant, repo, branch, workspace info. */
  const context: ExecutionContext;

  /**
   * Escalate to admin context (bypasses RLS).
   * Requires `requiresAdmin: true` in function .node.yaml metadata.
   */
  function asAdmin(): typeof raisin.admin;
}

// ==========================================================================
// Transaction (returned by raisin.nodes.beginTransaction())
// ==========================================================================

interface Transaction {
  create(workspace: string, parentPath: string, data: NodeCreateData): any;
  add(workspace: string, data: NodeCreateData): any;
  put(workspace: string, data: NodeCreateData): void;
  upsert(workspace: string, data: NodeCreateData): void;
  createDeep(workspace: string, parentPath: string, data: NodeCreateData, parentNodeType?: string): any;
  upsertDeep(workspace: string, data: NodeCreateData, parentNodeType?: string): void;
  update(workspace: string, path: string, data: Partial<NodeCreateData>): void;
  delete(workspace: string, path: string): void;
  deleteById(workspace: string, id: string): void;
  get(workspace: string, id: string): RaisinNode | null;
  getByPath(workspace: string, path: string): RaisinNode | null;
  listChildren(workspace: string, parentPath: string): RaisinNode[];
  updateProperty(workspace: string, nodePath: string, propertyPath: string, value: any): void;
  setActor(actor: string): void;
  setMessage(message: string): void;
  commit(): void;
  rollback(): void;
}

// ==========================================================================
// Console (logging)
// ==========================================================================

declare namespace console {
  function log(...args: any[]): void;
  function debug(...args: any[]): void;
  function warn(...args: any[]): void;
  function error(...args: any[]): void;
}

// ==========================================================================
// W3C Fetch API (built-in — no import needed)
// ==========================================================================

declare function fetch(input: string | Request, init?: RequestInit): Promise<Response>;
declare function setTimeout(callback: () => void, ms?: number): number;
declare function clearTimeout(id: number): void;
declare function setInterval(callback: () => void, ms?: number): number;
declare function clearInterval(id: number): void;

/** Standard function export pattern: module.exports = { handler }; */
declare var module: { exports: Record<string, any> };
