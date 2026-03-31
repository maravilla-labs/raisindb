// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! FormData class and fetch() function polyfill JavaScript

pub const JS_FETCH_FN: &str = r#"
// ============= FormData =============
class FormData {
    #entries = [];

    constructor() {
        // Note: Form parameter (HTMLFormElement) not supported
    }

    append(name, value, filename) {
        this.#entries.push({ name: String(name), value, filename });
    }

    delete(name) {
        this.#entries = this.#entries.filter(e => e.name !== name);
    }

    get(name) {
        const entry = this.#entries.find(e => e.name === name);
        return entry ? entry.value : null;
    }

    getAll(name) {
        return this.#entries.filter(e => e.name === name).map(e => e.value);
    }

    has(name) {
        return this.#entries.some(e => e.name === name);
    }

    set(name, value, filename) {
        this.delete(name);
        this.append(name, value, filename);
    }

    *entries() {
        for (const e of this.#entries) {
            yield [e.name, e.value];
        }
    }

    *keys() {
        for (const e of this.#entries) {
            yield e.name;
        }
    }

    *values() {
        for (const e of this.#entries) {
            yield e.value;
        }
    }

    [Symbol.iterator]() { return this.entries(); }

    forEach(callback, thisArg) {
        for (const entry of this.#entries) {
            callback.call(thisArg, entry.value, entry.name, this);
        }
    }

    // Internal: serialize for Rust
    _serialize() {
        return JSON.stringify(this.#entries);
    }
}

// ============= fetch() =============
async function fetch(input, init = {}) {
    // Build Request
    const request = input instanceof Request ? new Request(input, init) : new Request(input, init);

    // Check for abort before starting
    if (request.signal?.aborted) {
        throw request.signal.reason ?? new DOMException('The operation was aborted.', 'AbortError');
    }

    // Serialize body
    let bodyData = null;
    const body = request._getBody();
    if (body !== null && body !== undefined) {
        if (typeof body === 'string') {
            bodyData = { type: 'Text', data: body };
        } else if (body instanceof FormData) {
            bodyData = { type: 'FormData', data: body._serialize() };
        } else if (body instanceof ArrayBuffer || body instanceof Uint8Array) {
            const bytes = body instanceof Uint8Array ? body : new Uint8Array(body);
            bodyData = { type: 'ArrayBuffer', data: btoa(String.fromCharCode(...bytes)) };
        } else if (typeof body === 'object') {
            bodyData = { type: 'Json', data: body };
        }
    }

    // Build fetch request
    const fetchRequest = {
        url: request.url,
        method: request.method,
        headers: request.headers._toObject(),
        body: bodyData,
        signal_id: request.signal?._id ?? null,
        timeout_ms: null,
        mode: request.mode,
        credentials: request.credentials,
        cache: request.cache,
        redirect: request.redirect
    };

    // Call Rust
    const resultJson = __raisin_internal.fetch_request(JSON.stringify(fetchRequest));
    const result = JSON.parse(resultJson);

    // Check for errors
    if (result.error) {
        if (result.error === 'AbortError') {
            throw new DOMException(result.message || 'The operation was aborted.', 'AbortError');
        }
        if (result.error === 'TypeError') {
            throw new TypeError(result.message);
        }
        if (result.error === 'TimeoutError') {
            throw new DOMException(result.message || 'The operation timed out.', 'TimeoutError');
        }
        throw new TypeError(result.message || 'fetch failed');
    }

    // Build Response
    return new Response(null, {
        status: result.status,
        statusText: result.status_text,
        headers: result.headers,
        _meta: result
    });
}

// ============= Export to global =============
globalThis.fetch = fetch;
globalThis.Request = Request;
globalThis.Response = Response;
globalThis.Headers = Headers;
globalThis.FormData = FormData;
globalThis.AbortController = AbortController;
globalThis.AbortSignal = AbortSignal;
globalThis.ReadableStream = ReadableStream;
globalThis.DOMException = DOMException;
"#;
