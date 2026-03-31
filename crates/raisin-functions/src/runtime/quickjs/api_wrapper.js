// RaisinDB JavaScript API wrapper
// This code is evaluated at runtime to create the public globalThis.raisin API.
// It wraps internal __raisin_internal.* functions (which return JSON strings)
// into a developer-friendly API.

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
        const result = __raisin_internal.resource_getBinary(storageKey);
        if (result.startsWith('error:')) {
            throw new Error(result.substring(6));
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
            const result = __raisin_internal.node_addResource(
                workspace, this.path, path, JSON.stringify(uploadData)
            );
            const parsed = JSON.parse(result);
            if (parsed.error) throw new Error(parsed.error);
            return parsed;
        }
    };
}

globalThis.raisin = {
    nodes: {
        get: (workspace, path) => {
            const data = JSON.parse(__raisin_internal.nodes_get(workspace, path));
            return wrapNode(data, workspace);
        },
        getById: (workspace, id) => {
            const data = JSON.parse(__raisin_internal.nodes_getById(workspace, id));
            return wrapNode(data, workspace);
        },
        create: (workspace, parent, data) => {
            const result = JSON.parse(__raisin_internal.nodes_create(workspace, parent, JSON.stringify(data)));
            return wrapNode(result, workspace);
        },
        update: (workspace, path, data) => {
            const result = JSON.parse(__raisin_internal.nodes_update(workspace, path, JSON.stringify(data)));
            return wrapNode(result, workspace);
        },
        delete: (workspace, path) => __raisin_internal.nodes_delete(workspace, path),
        updateProperty: (workspace, nodePath, propertyPath, value) => __raisin_internal.nodes_updateProperty(workspace, nodePath, propertyPath, JSON.stringify(value)),
        move: (workspace, nodePath, newParentPath) => {
            const result = JSON.parse(__raisin_internal.nodes_move(workspace, nodePath, newParentPath));
            return wrapNode(result, workspace);
        },
        query: (workspace, query) => {
            const results = JSON.parse(__raisin_internal.nodes_query(workspace, JSON.stringify(query)));
            return results.map(n => wrapNode(n, workspace));
        },
        getChildren: (workspace, path, limit) => {
            const results = JSON.parse(__raisin_internal.nodes_getChildren(workspace, path, limit));
            return results.map(n => wrapNode(n, workspace));
        },
        // Transaction API - returns a context object with node operations
        beginTransaction: () => {
            const txId = __raisin_internal.tx_begin();
            if (txId.startsWith('error:')) {
                throw new Error(txId.substring(6));
            }
            return {
                // Create node under parent path (auto-generates ID)
                create: (workspace, parentPath, data) => {
                    const result = __raisin_internal.tx_create(txId, workspace, parentPath, JSON.stringify(data));
                    const parsed = JSON.parse(result);
                    if (parsed.error) throw new Error(parsed.error);
                    return parsed;
                },
                // Add node with explicit path (auto-generates ID if not provided)
                add: (workspace, data) => {
                    const result = __raisin_internal.tx_add(txId, workspace, JSON.stringify(data));
                    const parsed = JSON.parse(result);
                    if (parsed.error) throw new Error(parsed.error);
                    return parsed;
                },
                // Put node by ID (create or update, auto-generates ID if not provided)
                put: (workspace, data) => {
                    const result = __raisin_internal.tx_put(txId, workspace, JSON.stringify(data));
                    if (result.startsWith('{"error":')) {
                        throw new Error(JSON.parse(result).error);
                    }
                },
                // Upsert node by path (create or update, auto-generates ID if not provided)
                upsert: (workspace, data) => {
                    const result = __raisin_internal.tx_upsert(txId, workspace, JSON.stringify(data));
                    if (result.startsWith('{"error":')) {
                        throw new Error(JSON.parse(result).error);
                    }
                },
                // Create node with deep parent creation (auto-creates parent folders)
                createDeep: (workspace, parentPath, data, parentNodeType = 'raisin:Folder') => {
                    const result = __raisin_internal.tx_create_deep(txId, workspace, parentPath, JSON.stringify(data), parentNodeType);
                    const parsed = JSON.parse(result);
                    if (parsed.error) throw new Error(parsed.error);
                    return parsed;
                },
                // Upsert node with deep parent creation (auto-creates parent folders)
                upsertDeep: (workspace, data, parentNodeType = 'raisin:Folder') => {
                    const result = __raisin_internal.tx_upsert_deep(txId, workspace, JSON.stringify(data), parentNodeType);
                    if (result !== 'true') {
                        const parsed = JSON.parse(result);
                        if (parsed.error) throw new Error(parsed.error);
                    }
                },
                // Update existing node
                update: (workspace, path, data) => {
                    const result = __raisin_internal.tx_update(txId, workspace, path, JSON.stringify(data));
                    if (result.startsWith('{"error":')) {
                        throw new Error(JSON.parse(result).error);
                    }
                },
                // Delete node by path
                delete: (workspace, path) => __raisin_internal.tx_delete(txId, workspace, path),
                // Delete node by ID
                deleteById: (workspace, id) => __raisin_internal.tx_delete_by_id(txId, workspace, id),
                // Get node by ID
                get: (workspace, id) => {
                    const result = __raisin_internal.tx_get(txId, workspace, id);
                    return result === 'null' ? null : JSON.parse(result);
                },
                // Get node by path
                getByPath: (workspace, path) => {
                    const result = __raisin_internal.tx_get_by_path(txId, workspace, path);
                    return result === 'null' ? null : JSON.parse(result);
                },
                // List children of a node
                listChildren: (workspace, parentPath) => JSON.parse(__raisin_internal.tx_list_children(txId, workspace, parentPath)),
                // NOTE: tx.move() is intentionally NOT supported.
                // Move requires target parent to be committed, which conflicts with transaction semantics.
                // For "move" within a transaction, use: tx.delete(oldPath) + tx.add(newPath, { id: sameId, ... })
                // Update a single property
                updateProperty: (workspace, nodePath, propertyPath, value) => {
                    const result = __raisin_internal.tx_update_property(txId, workspace, nodePath, propertyPath, JSON.stringify(value));
                    if (result.startsWith('{"error":')) {
                        throw new Error(JSON.parse(result).error);
                    }
                },
                // Set actor for commit
                setActor: (actor) => __raisin_internal.tx_set_actor(txId, actor),
                // Set message for commit
                setMessage: (message) => __raisin_internal.tx_set_message(txId, message),
                // Commit transaction
                commit: () => {
                    const success = __raisin_internal.tx_commit(txId);
                    if (!success) throw new Error('Transaction commit failed');
                },
                // Rollback transaction
                rollback: () => {
                    const success = __raisin_internal.tx_rollback(txId);
                    if (!success) throw new Error('Transaction rollback failed');
                }
            };
        }
    },
    sql: {
        query: (sql, params) => JSON.parse(__raisin_internal.sql_query(sql, params ? JSON.stringify(params) : null)),
        execute: (sql, params) => __raisin_internal.sql_execute(sql, params ? JSON.stringify(params) : null)
    },
    http: {
        fetch: (url, options) => JSON.parse(__raisin_internal.http_fetch(url, options ? JSON.stringify(options) : null))
    },
    events: {
        emit: (eventType, data) => __raisin_internal.events_emit(eventType, JSON.stringify(data))
    },
    ai: {
        completion: (request) => {
            const result = __raisin_internal.ai_completion(JSON.stringify(request));
            const parsed = JSON.parse(result);
            if (parsed.error) {
                throw new Error(parsed.error);
            }
            return parsed;
        },
        embed: (request) => {
            const result = __raisin_internal.ai_embed(JSON.stringify(request));
            const parsed = JSON.parse(result);
            if (parsed.error) {
                throw new Error(parsed.error);
            }
            return parsed;
        },
        listModels: () => JSON.parse(__raisin_internal.ai_listModels()),
        getDefaultModel: (useCase) => __raisin_internal.ai_getDefaultModel(useCase)
    },
    functions: {
        execute: (functionPath, args, context) => JSON.parse(__raisin_internal.functions_execute(functionPath, JSON.stringify(args), JSON.stringify(context))),
        call: (functionPath, args) => {
            if (typeof __raisin_internal.functions_call !== 'function') {
                throw new Error('raisin.functions.call() is not available — server binary may need rebuild: cargo build --release --package raisin-server --features "storage-rocksdb,websocket,pgwire"');
            }
            return JSON.parse(__raisin_internal.functions_call(functionPath, JSON.stringify(args)));
        }
    },
    tasks: {
        create: (request) => JSON.parse(__raisin_internal.task_create(JSON.stringify(request)))
    },
    crypto: {
        uuid: () => __raisin_internal.crypto_uuid()
    },
    pdf: {
        // Extract text from PDF - base64Data is the PDF content
        // Returns { text, pages, isScanned, pageCount }
        extractText: (base64Data) => {
            const result = __raisin_internal.pdf_extractText(base64Data);
            if (result.startsWith('{"error":')) {
                const parsed = JSON.parse(result);
                throw new Error(parsed.error);
            }
            return JSON.parse(result);
        },
        // Get page count from PDF
        getPageCount: (base64Data) => {
            const result = __raisin_internal.pdf_getPageCount(base64Data);
            if (result < 0) {
                throw new Error('Failed to get PDF page count');
            }
            return result;
        },
        // OCR - Extract text from image using Tesseract
        // base64Data: base64-encoded image (PNG, JPEG, TIFF, etc.)
        // options: { languages: ["eng"], preserveLayout: false }
        // Returns { text, available }
        ocr: (base64Data, options) => {
            const optionsStr = options ? JSON.stringify(options) : '{}';
            const result = __raisin_internal.pdf_ocr(base64Data, optionsStr);
            if (result.startsWith('{"error":')) {
                const parsed = JSON.parse(result);
                throw new Error(parsed.error);
            }
            return JSON.parse(result);
        },
        // Async: Process PDF from storage key (no base64 overhead)
        // storageKey: storage key from resource metadata (e.g., "uploads/tenant/doc.pdf")
        // options: { ocr: true, ocrLanguages: ["eng"], generateThumbnail: true, thumbnailWidth: 200 }
        // Returns { text, pageCount, isScanned, ocrUsed, extractionMethod, thumbnail }
        processFromStorage: async (storageKey, options) => {
            const optionsStr = options ? JSON.stringify(options) : '{}';
            const result = __raisin_internal.pdf_processFromStorage(storageKey, optionsStr);
            if (result.startsWith('{"error":')) {
                const parsed = JSON.parse(result);
                throw new Error(parsed.error);
            }
            return JSON.parse(result);
        }
    },
    // Admin escalation - returns a new raisin object with admin context
    // Requires requiresAdmin: true in function metadata
    asAdmin: function() {
        // Check if function has permission to escalate
        if (!__raisin_internal.allows_admin_escalation()) {
            throw new Error("Function does not have permission to escalate to admin context. Set 'requiresAdmin: true' in function metadata.");
        }

        // Return a new raisin-like object that uses admin callbacks
        // The admin callbacks bypass RLS filtering
        return {
            nodes: {
                get: (workspace, path) => JSON.parse(__raisin_internal.admin_nodes_get(workspace, path)),
                getById: (workspace, id) => JSON.parse(__raisin_internal.admin_nodes_getById(workspace, id)),
                create: (workspace, parent, data) => JSON.parse(__raisin_internal.admin_nodes_create(workspace, parent, JSON.stringify(data))),
                update: (workspace, path, data) => JSON.parse(__raisin_internal.admin_nodes_update(workspace, path, JSON.stringify(data))),
                delete: (workspace, path) => __raisin_internal.admin_nodes_delete(workspace, path),
                updateProperty: (workspace, nodePath, propertyPath, value) => __raisin_internal.admin_nodes_updateProperty(workspace, nodePath, propertyPath, JSON.stringify(value)),
                query: (workspace, query) => JSON.parse(__raisin_internal.admin_nodes_query(workspace, JSON.stringify(query))),
                getChildren: (workspace, path, limit) => JSON.parse(__raisin_internal.admin_nodes_getChildren(workspace, path, limit)),
            },
            sql: {
                query: (sql, params) => JSON.parse(__raisin_internal.admin_sql_query(sql, params ? JSON.stringify(params) : null)),
                execute: (sql, params) => __raisin_internal.admin_sql_execute(sql, params ? JSON.stringify(params) : null)
            },
            // http, events, ai, functions, tasks remain the same - no RLS implications
            http: globalThis.raisin.http,
            events: globalThis.raisin.events,
            ai: globalThis.raisin.ai,
            functions: globalThis.raisin.functions,
            tasks: globalThis.raisin.tasks,
            // context remains the same
            context: globalThis.raisin.context
        };
    }
};
