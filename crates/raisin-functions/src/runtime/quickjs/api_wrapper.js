// RaisinDB JavaScript API wrapper
// This code is evaluated at runtime to create the public globalThis.raisin API.
//
// Every raisin.* method dispatches through __raisin_call (a host function
// registered by gateway.rs) into the SHARED bindings registry — the same
// single-definition-per-method registry the Starlark runtime uses. The only
// other host functions are the runtime-local __raisin_internal.temp_* helpers
// (per-execution temp files backing the Resource class) and the W3C fetch
// plumbing.
//
// The per-method bodies below are the FROZEN public surface contract:
// method names, argument shapes, return shapes and — critically — the
// per-method error conventions (throw vs null vs [] vs sentinel vs returned
// error object). Do not "unify" them; the application ecosystem depends on
// each one.

// Dispatch a method through the shared bindings registry.
// Returns the parsed result; a failed call yields the gateway's error
// envelope { error: true, message: "..." } (it never throws for API errors).
const __call = (method, args) => JSON.parse(__raisin_call(method, JSON.stringify(args)));

// True only for the gateway's error envelope. Legitimate API payloads never
// carry a top-level `error: true` with a string `message`.
const __isErr = (r) => !!r && typeof r === 'object' && r.error === true && typeof r.message === 'string';

// Resource class - represents a resource (file/binary) from node properties
class Resource {
    constructor(data, context) {
        this._uuid = data.uuid;
        this._name = data.name;
        this._size = data.size;
        this._mimeType = data.mime_type;
        this._metadata = data.metadata || {};
        this._context = context;  // { workspace, nodePath, propertyPath }
        this._tempHandle = data._tempHandle || null;  // For processed resources
    }

    // Metadata accessors
    get uuid() { return this._uuid; }
    get name() { return this._name; }
    get size() { return this._size; }
    get mimeType() { return this._mimeType; }
    get metadata() { return this._metadata; }

    // Async: Get binary data as base64
    async getBinary() {
        // If this is a temp resource (from resize), use temp handle
        if (this._tempHandle) {
            const result = __raisin_internal.temp_getBinary(this._tempHandle);
            if (result.startsWith('error:')) {
                throw new Error(result.substring(6));
            }
            return result;
        }

        // Otherwise get from storage
        const storageKey = this._metadata?.storage_key;
        if (!storageKey) {
            throw new Error('Resource has no storage_key in metadata');
        }
        const result = __call('resources_getBinary', [storageKey]);
        if (__isErr(result)) {
            throw new Error(result.message);
        }
        return result;
    }

    // Async: Get as data URL
    async toDataUrl() {
        const base64 = await this.getBinary();
        return 'data:' + this._mimeType + ';base64,' + base64;
    }

    // Async: Resize image using ImageMagick, returns new Resource
    async resize(options = {}) {
        // Get binary data
        const base64 = await this.getBinary();

        // Create temp file from base64
        const tempHandle = __raisin_internal.temp_createFromBase64(base64, this._mimeType, this._name);
        if (tempHandle.startsWith('error:')) {
            throw new Error(tempHandle.substring(6));
        }

        // Resize using ImageMagick
        const resizedHandle = __raisin_internal.temp_resize(tempHandle, JSON.stringify(options));
        if (resizedHandle.startsWith('error:')) {
            throw new Error(resizedHandle.substring(6));
        }

        // Get the new mime type
        const newMimeType = __raisin_internal.temp_getMimeType(resizedHandle);
        if (newMimeType.startsWith('error:')) {
            throw new Error(newMimeType.substring(6));
        }

        // Return new Resource with temp handle
        return new Resource({
            uuid: 'temp-' + Date.now(),
            name: this._name,
            size: null,  // Size unknown until read
            mime_type: newMimeType,
            metadata: {},
            _tempHandle: resizedHandle
        }, { ...this._context, tempHandle: resizedHandle });
    }

    // Async: Convert PDF page to image, returns new Resource
    // Options: { page: 0, maxWidth: 800, format: 'jpeg', quality: 85 }
    async toImage(options = {}) {
        if (!this._mimeType?.includes('pdf')) {
            throw new Error('toImage() only works with PDF files');
        }

        // Get binary data
        const base64 = await this.getBinary();

        // Create temp file from base64
        const tempHandle = __raisin_internal.temp_createFromBase64(base64, this._mimeType, this._name);
        if (tempHandle.startsWith('error:')) {
            throw new Error(tempHandle.substring(6));
        }

        // Convert PDF page to image
        const imageHandle = __raisin_internal.temp_pdfToImage(tempHandle, JSON.stringify(options));
        if (imageHandle.startsWith('error:')) {
            throw new Error(imageHandle.substring(6));
        }

        // Get the new mime type
        const newMimeType = __raisin_internal.temp_getMimeType(imageHandle);
        if (newMimeType.startsWith('error:')) {
            throw new Error(newMimeType.substring(6));
        }

        // Generate new filename with image extension
        const ext = options.format === 'png' ? 'png' : (options.format === 'webp' ? 'webp' : 'jpg');
        let newName = this._name || 'page';
        if (newName.endsWith('.pdf')) {
            newName = newName.slice(0, -4) + '.' + ext;
        } else {
            newName = newName + '.' + ext;
        }

        // Return new Resource with temp handle
        return new Resource({
            uuid: 'temp-' + Date.now(),
            name: newName,
            size: null,
            mime_type: newMimeType,
            metadata: {},
            _tempHandle: imageHandle
        }, { ...this._context, tempHandle: imageHandle });
    }

    // Get page count for PDFs
    async getPageCount() {
        if (!this._mimeType?.includes('pdf')) {
            throw new Error('getPageCount() only works with PDF files');
        }

        const base64 = await this.getBinary();
        const tempHandle = __raisin_internal.temp_createFromBase64(base64, this._mimeType, this._name);
        if (tempHandle.startsWith('error:')) {
            throw new Error(tempHandle.substring(6));
        }

        const result = __raisin_internal.temp_pdfPageCount(tempHandle);
        if (typeof result === 'string' && result.startsWith('error:')) {
            throw new Error(result.substring(6));
        }
        return result;
    }

    // Async: Process PDF document - storage-key based (no base64 overhead)
    // Options: { ocr: true, ocrLanguages: ["eng"], generateThumbnail: true, thumbnailWidth: 200 }
    // Returns: { text, pageCount, isScanned, ocrUsed, extractionMethod, thumbnail }
    async processDocument(options = {}) {
        if (!this._mimeType?.includes('pdf')) {
            throw new Error('processDocument() only works with PDF files');
        }

        const storageKey = this._metadata?.storage_key;
        if (!storageKey) {
            throw new Error('Resource has no storage_key in metadata');
        }

        // Use the storage-key based API (no base64 overhead)
        return await raisin.pdf.processFromStorage(storageKey, options);
    }

    // Get storage key for this resource
    get storageKey() {
        return this._metadata?.storage_key || null;
    }
}

// Make Resource globally available
globalThis.Resource = Resource;

// Wrap node with resource helper methods
function wrapNode(nodeData, workspace) {
    if (!nodeData) return null;
    return {
        ...nodeData,

        // True if this node carries the given mixin (type-declared, transitively).
        // Reads the server-materialized `$mixins` set.
        hasMixin(mixinName) {
            const mixins = this.properties?.['$mixins'];
            return Array.isArray(mixins) && mixins.includes(mixinName);
        },

        // True if this node "is a" given type — its node_type, any `extends`
        // ancestor, or any effective mixin.
        // Reads the server-materialized `$supertypes` set.
        isNodeType(typeName) {
            if (this.node_type === typeName) return true;
            const supertypes = this.properties?.['$supertypes'];
            return Array.isArray(supertypes) && supertypes.includes(typeName);
        },

        // Get a Resource object from a property path (e.g., "./file" or "file")
        getResource(propertyPath) {
            const path = propertyPath.startsWith('./') ? propertyPath.slice(2) : propertyPath;
            const resourceData = this.properties?.[path];
            if (!resourceData) return null;
            return new Resource(resourceData, {
                workspace,
                nodePath: this.path,
                propertyPath: path
            });
        },

        // Upload new resource to node (returns resource metadata)
        async addResource(propertyPath, data) {
            const path = propertyPath.startsWith('./') ? propertyPath.slice(2) : propertyPath;
            // data can be: { base64, mimeType, name } or Resource
            let uploadData;
            if (data instanceof Resource) {
                // Get binary from existing resource
                const base64 = await data.getBinary();
                uploadData = { base64, mimeType: data.mimeType, name: data.name };
            } else if (typeof data === 'string') {
                uploadData = { base64: data, mimeType: 'application/octet-stream' };
            } else {
                uploadData = data;
            }
            const parsed = __call('nodes_addResource', [workspace, this.path, path, uploadData]);
            if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
            return parsed;
        }
    };
}

globalThis.raisin = {
    nodes: {
        // Reads swallow storage errors: a failed get resolves to null.
        get: (workspace, path) => {
            const data = __call('nodes_get', [workspace, path]);
            return wrapNode(__isErr(data) ? null : data, workspace);
        },
        getById: (workspace, id) => {
            const data = __call('nodes_getById', [workspace, id]);
            return wrapNode(__isErr(data) ? null : data, workspace);
        },
        // List a node's revision history (git-style "file history"), newest first.
        // Returns plain revision entries ({revision, updated_at, updated_by, deleted}),
        // not node objects. Use the `revision` with an at-revision read to fetch a snapshot.
        history: (workspace, id, limit) => {
            const results = __call('nodes_history', [workspace, id, limit]);
            return __isErr(results) ? [] : results;
        },
        create: (workspace, parent, data) => {
            const result = __call('nodes_create', [workspace, parent, data]);
            // A rejected create (permission denied, workspace allowlist, path
            // conflict, validation) comes back as an error envelope — it must
            // surface as an exception, never as a success-shaped node.
            if (result && result.error) throw new Error(result.message || result.error);
            return wrapNode(result, workspace);
        },
        // Create a node under parentPath, auto-creating any missing ancestor folders.
        // Wraps a single-shot transaction (begin -> createDeep -> commit) so it can be
        // called directly on raisin.nodes without managing a transaction by hand.
        createDeep: (workspace, parentPath, data, parentNodeType = 'raisin:Folder') => {
            const tx = globalThis.raisin.nodes.beginTransaction();
            try {
                const result = tx.createDeep(workspace, parentPath, data, parentNodeType);
                tx.commit();
                return wrapNode(result, workspace);
            } catch (e) {
                try { tx.rollback(); } catch (_) {}
                throw e;
            }
        },
        // Upsert a node by path (create-or-update), auto-creating any missing ancestor folders.
        upsertDeep: (workspace, data, parentNodeType = 'raisin:Folder') => {
            const tx = globalThis.raisin.nodes.beginTransaction();
            try {
                tx.upsertDeep(workspace, data, parentNodeType);
                tx.commit();
            } catch (e) {
                try { tx.rollback(); } catch (_) {}
                throw e;
            }
        },
        update: (workspace, path, data) => {
            const result = __call('nodes_update', [workspace, path, data]);
            if (result && result.error) throw new Error(result.message || result.error);
            return wrapNode(result, workspace);
        },
        delete: (workspace, path) => {
            const parsed = __call('nodes_delete', [workspace, path]);
            if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
            return true;
        },
        updateProperty: (workspace, nodePath, propertyPath, value) => {
            const parsed = __call('nodes_updateProperty', [workspace, nodePath, propertyPath, value]);
            if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
            return true;
        },
        move: (workspace, nodePath, newParentPath) => {
            const result = __call('nodes_move', [workspace, nodePath, newParentPath]);
            if (result && result.error) throw new Error(result.message || result.error);
            return wrapNode(result, workspace);
        },
        // Reads swallow storage errors: a failed query resolves to [].
        query: (workspace, query) => {
            const results = __call('nodes_query', [workspace, query]);
            return (__isErr(results) ? [] : results).map(n => wrapNode(n, workspace));
        },
        getChildren: (workspace, path, limit) => {
            const results = __call('nodes_getChildren', [workspace, path, limit]);
            return (__isErr(results) ? [] : results).map(n => wrapNode(n, workspace));
        },
        // Editorial ordering. You name a position or a neighbour; the ordering
        // index mints the fractional order key. Children are named, not paths.
        reorderChild: (workspace, parentPath, childName, position) => {
            const parsed = __call('nodes_reorderChild', [workspace, parentPath, childName, position]);
            if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
            return true;
        },
        moveChildBefore: (workspace, parentPath, childName, beforeChildName) => {
            const parsed = __call('nodes_moveChildBefore', [workspace, parentPath, childName, beforeChildName]);
            if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
            return true;
        },
        moveChildAfter: (workspace, parentPath, childName, afterChildName) => {
            const parsed = __call('nodes_moveChildAfter', [workspace, parentPath, childName, afterChildName]);
            if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
            return true;
        },
        // Replay a parent's child order from sourceBranch onto targetBranch.
        applyChildOrder: (workspace, parentPath, sourceBranch, targetBranch) => {
            const parsed = __call('nodes_applyChildOrder', [workspace, parentPath, sourceBranch, targetBranch]);
            if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
            return true;
        },
        // Transaction API - returns a context object with node operations
        beginTransaction: () => {
            const txId = __call('tx_begin', []);
            if (__isErr(txId)) {
                throw new Error(txId.message);
            }
            return {
                // Create node under parent path (auto-generates ID)
                create: (workspace, parentPath, data) => {
                    const parsed = __call('tx_create', [txId, workspace, parentPath, data]);
                    if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
                    return parsed;
                },
                // Add node with explicit path (auto-generates ID if not provided)
                add: (workspace, data) => {
                    const parsed = __call('tx_add', [txId, workspace, data]);
                    if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
                    return parsed;
                },
                // Put node by ID (create or update, auto-generates ID if not provided)
                put: (workspace, data) => {
                    const result = __call('tx_put', [txId, workspace, data]);
                    if (__isErr(result)) {
                        throw new Error(result.message);
                    }
                },
                // Upsert node by path (create or update, auto-generates ID if not provided)
                upsert: (workspace, data) => {
                    const result = __call('tx_upsert', [txId, workspace, data]);
                    if (__isErr(result)) {
                        throw new Error(result.message);
                    }
                },
                // Create node with deep parent creation (auto-creates parent folders)
                createDeep: (workspace, parentPath, data, parentNodeType = 'raisin:Folder') => {
                    const parsed = __call('tx_createDeep', [txId, workspace, parentPath, data, parentNodeType]);
                    if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
                    return parsed;
                },
                // Upsert node with deep parent creation (auto-creates parent folders)
                upsertDeep: (workspace, data, parentNodeType = 'raisin:Folder') => {
                    const result = __call('tx_upsertDeep', [txId, workspace, data, parentNodeType]);
                    if (result !== true && result && result.error) {
                        throw new Error(result.message || result.error);
                    }
                },
                // Update existing node
                update: (workspace, path, data) => {
                    const result = __call('tx_update', [txId, workspace, path, data]);
                    if (__isErr(result)) {
                        throw new Error(result.message);
                    }
                },
                // Delete node by path (errors swallowed to false, like commit-time bools)
                delete: (workspace, path) => {
                    const r = __call('tx_delete', [txId, workspace, path]);
                    return __isErr(r) ? false : true;
                },
                // Delete node by ID (errors swallowed to false)
                deleteById: (workspace, id) => {
                    const r = __call('tx_deleteById', [txId, workspace, id]);
                    return __isErr(r) ? false : true;
                },
                // Get node by ID (errors swallowed to null)
                get: (workspace, id) => {
                    const result = __call('tx_get', [txId, workspace, id]);
                    return __isErr(result) ? null : result;
                },
                // Get node by path (errors swallowed to null)
                getByPath: (workspace, path) => {
                    const result = __call('tx_getByPath', [txId, workspace, path]);
                    return __isErr(result) ? null : result;
                },
                // List children of a node (errors swallowed to [])
                listChildren: (workspace, parentPath) => {
                    const results = __call('tx_listChildren', [txId, workspace, parentPath]);
                    return __isErr(results) ? [] : results;
                },
                // NOTE: tx.move() is intentionally NOT supported (the registry's
                // tx_move exists for other surfaces but is not exposed here).
                // Move requires target parent to be committed, which conflicts with transaction semantics.
                // For "move" within a transaction, use: tx.delete(oldPath) + tx.add(newPath, { id: sameId, ... })
                // Update a single property
                updateProperty: (workspace, nodePath, propertyPath, value) => {
                    const result = __call('tx_updateProperty', [txId, workspace, nodePath, propertyPath, value]);
                    if (__isErr(result)) {
                        throw new Error(result.message);
                    }
                },
                // Set actor for commit (returns bool, errors swallowed to false)
                setActor: (actor) => {
                    const r = __call('tx_setActor', [txId, actor]);
                    return __isErr(r) ? false : true;
                },
                // Set message for commit (returns bool, errors swallowed to false)
                setMessage: (message) => {
                    const r = __call('tx_setMessage', [txId, message]);
                    return __isErr(r) ? false : true;
                },
                // Commit transaction
                commit: () => {
                    const r = __call('tx_commit', [txId]);
                    if (__isErr(r)) throw new Error('Transaction commit failed');
                },
                // Rollback transaction
                rollback: () => {
                    const r = __call('tx_rollback', [txId]);
                    if (__isErr(r)) throw new Error('Transaction rollback failed');
                }
            };
        }
    },
    sql: {
        // A failed query does NOT throw — it resolves to { error, rows: [] }.
        query: (sql, params) => {
            const r = __call('sql_query', [sql, Array.isArray(params) ? params : []]);
            return __isErr(r) ? { error: r.message, rows: [] } : r;
        },
        // A failed execute does NOT throw — it resolves to -1.
        execute: (sql, params) => {
            const r = __call('sql_execute', [sql, Array.isArray(params) ? params : []]);
            return __isErr(r) ? -1 : r;
        }
    },
    http: {
        // fetch(url, { method, headers, body, ... }). A failed request does
        // NOT throw — it resolves to { error, status: 0, ok: false }.
        fetch: (url, options) => {
            const opts = options || {};
            const method = typeof opts.method === 'string' ? opts.method : 'GET';
            const r = __call('http_request', [method, url, opts]);
            return __isErr(r) ? { error: r.message, status: 0, ok: false } : r;
        }
    },
    events: {
        // Returns bool; a failed emit resolves to false (never throws).
        emit: (eventType, data) => {
            const r = __call('events_emit', [eventType, data]);
            return __isErr(r) ? false : true;
        }
    },
    ai: {
        completion: (request) => {
            const parsed = __call('ai_completion', [request]);
            if (parsed && parsed.error) {
                throw new Error(parsed.message || parsed.error);
            }
            return parsed;
        },
        embed: (request) => {
            const parsed = __call('ai_embed', [request]);
            if (parsed && parsed.error) {
                throw new Error(parsed.message || parsed.error);
            }
            return parsed;
        },
        // Errors swallowed to [].
        listModels: () => {
            const models = __call('ai_listModels', []);
            return __isErr(models) ? [] : models;
        },
        // Returns "" when no default is configured (or on error) — never null.
        getDefaultModel: (useCase) => {
            const r = __call('ai_getDefaultModel', [useCase]);
            return (__isErr(r) || r === null || r === undefined) ? '' : r;
        }
    },
    functions: {
        // A failed execute does NOT throw — the { error } object is returned.
        execute: (functionPath, args, context) => {
            const r = __call('functions_execute', [functionPath, args, context]);
            return __isErr(r) ? { error: r.message } : r;
        },
        // Simple function-to-function call; same returned-error-object semantics.
        call: (functionPath, args) => {
            const r = __call('functions_call', [functionPath, args]);
            return __isErr(r) ? { error: r.message } : r;
        }
    },
    flows: {
        // Start a raisin:Flow by path (fire-and-forget). Returns
        // { instance_id, job_id, status: "queued" } - poll the flow
        // instance API to observe progress.
        run: (flowPath, input) => {
            const parsed = __call('flows_run', [flowPath, input || {}]);
            if (parsed && parsed.error) {
                throw new Error(parsed.message || parsed.error);
            }
            return parsed;
        }
    },
    branches: {
        // Per-node diff of `branch` relative to `baseBranch`'s merge-base:
        // exactly which nodes were added / modified / deleted since the two
        // branches diverged. Returns
        // { common_ancestor, added: [...], modified: [...], deleted: [...] }.
        diff: (branch, baseBranch) => {
            const r = __call('branches_diff', [branch, baseBranch]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        },
        // Branch divergence (commits ahead/behind, like Git's tracking info).
        // Returns { ahead, behind, common_ancestor }.
        compare: (branch, baseBranch) => {
            const r = __call('branches_compare', [branch, baseBranch]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        },
        // Copy a node set from sourceBranch onto targetBranch (branch
        // promotion): node ids preserved, one atomic commit on the target.
        // opts: { workspace, roots: [paths], recursive? (default true),
        // deleteMissing? (default false) }.
        // Returns { copied, deleted, revision, changes: [...] }.
        copyNodes: (sourceBranch, targetBranch, opts) => {
            const r = __call('branches_copyNodes', [sourceBranch, targetBranch, opts || {}]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        }
    },
    scheduler: {
        // Schedule a one-shot invocation of a function or flow at a fixed
        // time. request: { targetKind: "function"|"flow", targetPath,
        // input?, runAt (RFC3339 - a past time fires immediately),
        // externalKey?, branch?, workspace?, maxRetries? (default 0) }.
        // Returns { job_id, invocation_id, status: "scheduled", run_at }.
        schedule: (request) => {
            const r = __call('scheduler_schedule', [request || {}]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        },
        // Cancel a pending scheduled invocation by job id or external key.
        // Returns { job_id, status: "cancelled" }.
        cancel: (jobIdOrKey) => {
            const r = __call('scheduler_cancel', [jobIdOrKey]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        },
        // List this repository's scheduled invocations.
        // filter: { externalKey?, status? }. Returns { invocations: [...] }.
        list: (filter) => {
            const r = __call('scheduler_list', [filter || {}]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        },
        // Fetch a single scheduled invocation by job id or external key.
        get: (jobIdOrKey) => {
            const r = __call('scheduler_get', [jobIdOrKey]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        }
    },
    tasks: {
        // A failed create does NOT throw — the { error } object is returned.
        create: (request) => {
            const r = __call('tasks_create', [request]);
            return __isErr(r) ? { error: r.message } : r;
        },
        // complete(taskId, response?) -> { id, task_id, task_path, status,
        //   responded_at, flow }. Marks the task completed AND resumes the
        //   owning flow (feeding `response` as __human_response); `flow` is
        //   { instance_id, job_id } for a flow-owned task, else null. Validates
        //   the caller is the assignee (or an admin). A failed completion
        //   (wrong assignee, already completed, flow resume failed) THROWS —
        //   silently dropping it would strand a parked flow forever.
        complete: (taskId, response) => {
            const r = __call('tasks_complete', [taskId, response === undefined ? {} : response]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        }
    },
    // Send a system notification. Creates a raisin:Notification in the
    // recipient's `{recipient}/notifications` folder (which must already exist).
    // options: { title (required), body?, recipient (path) | recipientId (uuid),
    //   priority?, type?, link?, data? }. Returns
    // { success, notification_id, notification_path }; a failed send THROWS
    // (missing folder, unknown recipient, validation) — a dropped notification
    // is a real error the caller should see, matching scheduler/locks.
    notify: (options) => {
        const r = __call('notify_send', [options]);
        if (r && r.error) throw new Error(r.message || r.error);
        return r;
    },
    crypto: {
        uuid: () => __call('crypto_uuid', []),
        // verifyJwt(token, opts?) -> { valid, claims?, error? }
        // opts: { jwks_url?, issuer?, audience?, algorithms? }. The JWKS is
        // fetched from jwks_url whose host must be authorized by the function's
        // network policy (else the call is refused before any socket is opened).
        // An invalid token resolves to { valid:false, error }; a policy denial or
        // unreachable JWKS throws. The token and claims are never logged.
        verifyJwt: (token, opts) => {
            const r = __call('crypto_verify_jwt', [token, opts === undefined ? null : opts]);
            if (r && r.error && r.valid === undefined) throw new Error(r.message || r.error);
            return r;
        }
    },
    // Atomic lease-locks (mutual exclusion with fencing tokens).
    // Requires the [locks] subsystem to be enabled in server config, else throws.
    locks: {
        // acquire(key, ttlMs, owner?) -> { acquired, key?, token?, expires_at_ms? }
        // Check `.acquired`: true with the fence token, or false on a lost tie-breaker.
        acquire: (key, ttlMs, owner) => {
            const r = __call('locks_acquire', [key, ttlMs, owner === undefined ? null : owner]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
        // release(key, token) -> bool (true if released; errors swallowed to false)
        release: (key, token) => {
            const r = __call('locks_release', [key, token]);
            return __isErr(r) ? false : r;
        },
        // renew(key, token, ttlMs) -> bool (false if the lease was lost)
        renew: (key, token, ttlMs) => {
            const r = __call('locks_renew', [key, token, ttlMs]);
            return __isErr(r) ? false : r;
        },
    },
    // Encrypted secret store (raisin.secrets.*).
    //
    // Gated by the function's `secret_policy` in its .node.yaml:
    //
    //   secret_policy:
    //     enabled: true
    //     allowed_names: ["stripe/*"]
    //
    // With no secret_policy the function has NO secret access at all — every
    // call below throws a policy_denied error naming the secret. That default
    // is deliberate: adapters run privileged and hold raisin.http, so an
    // ungated secrets binding would be a one-line exfiltration path.
    //
    // Every method THROWS on failure (denial, missing secret, unconfigured
    // store). None of them degrade to a falsy value: a silent null here would
    // be indistinguishable from "the credential is empty", and the function
    // would go on to authenticate with nothing.
    secrets: {
        // get(nameOrRef, version?) -> string (the plaintext)
        //
        // Accepts EITHER spelling of the same secret:
        //   raisin.secrets.get('stripe_key')
        //   raisin.secrets.get('secret://node/01H8XY.../api_key@1')
        //
        // The second is what the main flow hands you. A node property in a
        // field declared `encrypted: true` stores the REFERENCE, and node reads
        // never resolve it, so:
        //
        //   const node = raisin.nodes.get('data', '/connections/stripe');
        //   const key  = raisin.secrets.get(node.properties.api_key);
        //
        // Pass the string through as-is — do NOT strip `secret://` or the
        // `@version` suffix yourself. A name may contain `@` (ops@example.com),
        // so only a trailing all-digit run after the LAST `@` is a version, and
        // that rule is applied here by the same parser the storage layer uses.
        //
        // A version pinned in the reference IS honoured (secret://k@1 returns
        // version 1, not the latest) — which is what makes reading an older
        // node revision give the value that revision actually held. Passing a
        // `version` argument on top of a pinned reference THROWS: two stated
        // versions cannot both hold, and quietly picking one could hand you a
        // value that revision never had. Pass one or the other.
        get: (nameOrRef, version) => {
            const r = __call('secrets_get', [nameOrRef, version === undefined ? null : version]);
            if (__isErr(r)) throw new Error(r.message);
            return r;
        },
        // resolve(value) -> string
        // Returns the plaintext if `value` is a secret:// reference, or `value`
        // unchanged if it is not — so a config field that is a literal on one
        // deployment and a vaulted reference on another is read the same way:
        //
        //   const password = raisin.secrets.resolve(conn.password);
        //
        // A reference that FAILS to resolve throws; it never falls back to
        // returning the reference text, which would send `secret://...` to a
        // provider as a credential.
        resolve: (value) => {
            const r = __call('secrets_resolve', [value]);
            if (__isErr(r)) throw new Error(r.message);
            return r;
        },
        // put(name, value) -> { name, version }   (appends; never overwrites)
        // A bare name or an UNPINNED secret:// reference; a pinned one is
        // refused, since a write appends rather than replacing that version.
        put: (name, value) => {
            const r = __call('secrets_put', [name, value]);
            if (__isErr(r)) throw new Error(r.message);
            return r;
        },
        // list() -> [{ name, version, created_at, created_by, deleted, ... }]
        // Metadata only — never ciphertext or plaintext — and filtered to the
        // names this function's policy allows.
        list: () => {
            const r = __call('secrets_list', []);
            if (__isErr(r)) throw new Error(r.message);
            return r;
        },
        // rotate(name, newValue) -> { name, version }
        // An append stamped as a rotation: pinned secret://name@N references
        // keep resolving to the old version.
        rotate: (name, value) => {
            const r = __call('secrets_rotate', [name, value]);
            if (__isErr(r)) throw new Error(r.message);
            return r;
        },
        // delete(name) -> { name, version }  (appends a tombstone)
        delete: (name) => {
            const r = __call('secrets_delete', [name]);
            if (__isErr(r)) throw new Error(r.message);
            return r;
        },
    },
    // Counting reservations (claim N of M units without overselling).
    inventory: {
        // claim(pool, n, capacity) -> { claimed, remaining? }
        // Check `.claimed`: true with `.remaining`, or false when sold out.
        claim: (pool, n, capacity) => {
            const r = __call('inventory_claim', [pool, n, capacity]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
        // release(pool, n) -> number (new remaining count; -1 on error, never throws)
        release: (pool, n) => {
            const r = __call('inventory_release', [pool, n]);
            return __isErr(r) ? -1 : r;
        },
    },
    // Integration / mount ("connector") operations.
    integrations: {
        // sync_now(mountId, mode?) -> { job_id: string|null, status: "queued"|"already_running" }
        // Enqueues a deduped VirtualMountSync for the mount. `mode` defaults to
        // "delta" (pass "full" for a full re-sync). This is what a provider
        // webhook-refresh function calls to pull external changes on demand.
        sync_now: (mountId, mode) => {
            const r = __call('integrations_sync_now', [mountId, mode === undefined ? null : mode]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
        // camelCase alias for consistency with other raisin.* namespaces.
        syncNow: (mountId, mode) => {
            const r = __call('integrations_sync_now', [mountId, mode === undefined ? null : mode]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
    },
    // Native IMAP protocol operations (raisin.imap.*).
    // `conn` = { host, port, tls: true, username, password }. The password may
    // be an app password or an OAuth2 XOAUTH2 access token; it is never logged.
    // The connection's host:port must be authorized by the function's network
    // policy (e.g. allowed_urls: ["imaps://imap.example.org:993"]) or the call
    // is refused before any socket is opened.
    imap: {
        // fetchSince(conn, sinceUid, opts?) -> { messages, highestUid, uidvalidity }
        // opts: { mailbox?: string ("INBOX"), limit?: number (200, capped) }.
        // Only messages with UID > sinceUid are returned. `highestUid` is the
        // new cursor (unchanged when nothing is new); a changed `uidvalidity`
        // means the mailbox reset and the caller must full-resync.
        fetchSince: (conn, sinceUid, opts) => {
            const r = __call('imap_fetch_since', [conn, sinceUid, opts === undefined ? null : opts]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
        // listMailboxes(conn) -> [ { name, path, flags } ]
        listMailboxes: (conn) => {
            const r = __call('imap_list_mailboxes', [conn]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
        // fetchMessage(conn, uid, opts?) -> { headers, from, to, subject, date,
        //   text, html?, snippet, flags, message_id }. opts: { mailbox?: string }.
        fetchMessage: (conn, uid, opts) => {
            const r = __call('imap_fetch_message', [conn, uid, opts === undefined ? null : opts]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
    },
    // Transactional email (raisin.email.*).
    // The sender identity (from, replyTo) and the provider credential come from
    // the tenant's /config/email node, NOT from the caller — a function chooses
    // who receives a message, never who it appears to be from.
    email: {
        // send({ to, subject, text, html? }) -> { message_id, provider }
        // `to` is one address or an array of them. The receipt means the
        // provider ACCEPTED the message; delivery is a later, separate event.
        // Gated by the function's `email_policy` in its .node.yaml:
        //
        //   email_policy:
        //     enabled: true
        //     allowed_recipients: ["example.com", "*.example.com"]
        //
        // With no email_policy the function cannot send at all, and one
        // disallowed recipient refuses the whole message — there is no partial
        // send. Throws for that, when email is not configured/enabled for the
        // tenant, when the function's secret_policy does not grant the
        // configured credential, or when the provider rejects the send.
        send: (message) => {
            const r = __call('email_send', [message]);
            if (r && r.error) throw new Error(r.message || r.error);
            return r;
        },
    },
    pdf: {
        // Extract text from PDF - base64Data is the PDF content
        // Returns { text, pages, isScanned, pageCount }
        extractText: (base64Data) => {
            const r = __call('pdf_extractText', [base64Data]);
            if (r && r.error) {
                throw new Error(r.message || r.error);
            }
            return r;
        },
        // Get page count from PDF
        getPageCount: (base64Data) => {
            const r = __call('pdf_getPageCount', [base64Data]);
            if (typeof r !== 'number' || r < 0) {
                throw new Error('Failed to get PDF page count');
            }
            return r;
        },
        // OCR - Extract text from image using Tesseract
        // base64Data: base64-encoded image (PNG, JPEG, TIFF, etc.)
        // options: { languages: ["eng"], preserveLayout: false }
        // Returns { text, available }. When OCR is unavailable the result is
        // DATA ({ text: "", available: false, error }), not a thrown error.
        ocr: (base64Data, options) => {
            const r = __call('pdf_ocr', [base64Data, options || {}]);
            if (__isErr(r)) {
                throw new Error(r.message);
            }
            return r;
        },
        // Async: Process PDF from storage key (no base64 overhead)
        // storageKey: storage key from resource metadata (e.g., "uploads/tenant/doc.pdf")
        // options: { ocr: true, ocrLanguages: ["eng"], generateThumbnail: true, thumbnailWidth: 200 }
        // Returns { text, pageCount, isScanned, ocrUsed, extractionMethod, thumbnail }
        processFromStorage: async (storageKey, options) => {
            const r = __call('pdf_processFromStorage', [storageKey, options || {}]);
            if (r && r.error) {
                throw new Error('PDF processing failed: ' + (r.message || r.error));
            }
            return r;
        }
    },
    // Admin escalation - returns a new raisin object with admin context
    // Requires requiresAdmin: true in function metadata
    asAdmin: function() {
        // Check if function has permission to escalate
        if (__call('allowsAdminEscalation', []) !== true) {
            throw new Error("Function does not have permission to escalate to admin context. Set 'requiresAdmin: true' in function metadata.");
        }

        // Return a new raisin-like object that uses admin callbacks
        // The admin callbacks bypass RLS filtering.
        // Admin node reads do NOT wrapNode; reads swallow errors (null/[]),
        // writes throw — same conventions as the non-admin namespace.
        return {
            nodes: {
                get: (workspace, path) => {
                    const r = __call('admin_nodes_get', [workspace, path]);
                    return __isErr(r) ? null : r;
                },
                getById: (workspace, id) => {
                    const r = __call('admin_nodes_getById', [workspace, id]);
                    return __isErr(r) ? null : r;
                },
                create: (workspace, parent, data) => {
                    const result = __call('admin_nodes_create', [workspace, parent, data]);
                    if (result && result.error) throw new Error(result.message || result.error);
                    return result;
                },
                update: (workspace, path, data) => {
                    const result = __call('admin_nodes_update', [workspace, path, data]);
                    if (result && result.error) throw new Error(result.message || result.error);
                    return result;
                },
                delete: (workspace, path) => {
                    const parsed = __call('admin_nodes_delete', [workspace, path]);
                    if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
                    return true;
                },
                updateProperty: (workspace, nodePath, propertyPath, value) => {
                    const parsed = __call('admin_nodes_updateProperty', [workspace, nodePath, propertyPath, value]);
                    if (parsed && parsed.error) throw new Error(parsed.message || parsed.error);
                    return true;
                },
                query: (workspace, query) => {
                    const r = __call('admin_nodes_query', [workspace, query]);
                    return __isErr(r) ? [] : r;
                },
                getChildren: (workspace, path, limit) => {
                    const r = __call('admin_nodes_getChildren', [workspace, path, limit]);
                    return __isErr(r) ? [] : r;
                },
            },
            sql: {
                query: (sql, params) => {
                    const r = __call('admin_sql_query', [sql, Array.isArray(params) ? params : []]);
                    return __isErr(r) ? { error: r.message, rows: [] } : r;
                },
                execute: (sql, params) => {
                    const r = __call('admin_sql_execute', [sql, Array.isArray(params) ? params : []]);
                    return __isErr(r) ? -1 : r;
                }
            },
            // http, events, ai, functions, tasks, notify remain the same - no RLS implications
            http: globalThis.raisin.http,
            events: globalThis.raisin.events,
            ai: globalThis.raisin.ai,
            functions: globalThis.raisin.functions,
            tasks: globalThis.raisin.tasks,
            notify: globalThis.raisin.notify,
            // locks/inventory have no RLS implications - reuse the same managers
            locks: globalThis.raisin.locks,
            inventory: globalThis.raisin.inventory,
            // context remains the same
            context: globalThis.raisin.context
        };
    }
};
