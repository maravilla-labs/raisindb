// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Response class polyfill JavaScript

pub const JS_RESPONSE: &str = r#"
// ============= Response =============
class Response {
    #meta;
    #headers;
    #body;
    #bodyUsed = false;

    static error() {
        return new Response(null, {
            status: 0,
            statusText: '',
            _meta: { response_type: 'error' }
        });
    }

    static redirect(url, status = 302) {
        if (![301, 302, 303, 307, 308].includes(status)) {
            throw new RangeError('Invalid redirect status');
        }
        return new Response(null, {
            status,
            headers: { Location: url }
        });
    }

    static json(data, init = {}) {
        const body = JSON.stringify(data);
        const headers = new Headers(init.headers);
        if (!headers.has('content-type')) {
            headers.set('content-type', 'application/json');
        }
        return new Response(body, { ...init, headers });
    }

    constructor(body, init = {}) {
        this.#meta = init._meta ?? {
            status: init.status ?? 200,
            status_text: init.statusText ?? '',
            headers: {},
            url: '',
            redirected: false,
            response_type: 'default',
            stream_id: null
        };

        this.#headers = new Headers(init.headers ?? this.#meta.headers);

        // Body can be:
        // - null (but may have stream_id in meta)
        // - string
        // - ReadableStream (from Rust)
        // - stream_id (string, internal)
        //
        // IMPORTANT: Check stream_id FIRST because fetch() passes body=null
        // with the actual body in _meta.stream_id
        if (this.#meta.stream_id) {
            // Internal: body is a stream ID from Rust (from fetch())
            this.#body = new ReadableStream(this.#meta.stream_id);
        } else if (body === null || body === undefined) {
            this.#body = null;
        } else if (typeof body === 'string') {
            // Create a buffered stream from string
            this.#body = body;
        } else if (body instanceof ReadableStream) {
            this.#body = body;
        } else {
            this.#body = body;
        }
    }

    get ok() { return this.#meta.status >= 200 && this.#meta.status < 300; }
    get status() { return this.#meta.status; }
    get statusText() { return this.#meta.status_text || ''; }
    get headers() { return this.#headers; }
    get url() { return this.#meta.url || ''; }
    get redirected() { return this.#meta.redirected || false; }
    get type() { return this.#meta.response_type || 'basic'; }
    get bodyUsed() { return this.#bodyUsed; }

    get body() {
        if (this.#body instanceof ReadableStream) {
            return this.#body;
        }
        return null;
    }

    clone() {
        if (this.#bodyUsed) {
            throw new TypeError('Body has already been consumed');
        }
        // Note: This is a simplified clone that doesn't properly clone streams
        return new Response(this.#body, {
            status: this.#meta.status,
            statusText: this.#meta.status_text,
            headers: this.#headers,
            _meta: { ...this.#meta }
        });
    }

    async arrayBuffer() {
        if (this.#bodyUsed) throw new TypeError('Body has already been consumed');
        this.#bodyUsed = true;

        if (this.#body === null) {
            return new ArrayBuffer(0);
        }

        if (typeof this.#body === 'string') {
            return new TextEncoder().encode(this.#body).buffer;
        }

        if (this.#body instanceof ReadableStream) {
            const chunks = [];
            let totalLength = 0;

            for await (const chunk of this.#body) {
                chunks.push(chunk);
                totalLength += chunk.length;
            }

            const result = new Uint8Array(totalLength);
            let offset = 0;
            for (const chunk of chunks) {
                result.set(chunk, offset);
                offset += chunk.length;
            }

            return result.buffer;
        }

        throw new Error('Cannot convert body to ArrayBuffer');
    }

    async blob() {
        const buffer = await this.arrayBuffer();
        const contentType = this.#headers.get('content-type') ?? '';
        // Return a blob-like object
        return {
            type: contentType,
            size: buffer.byteLength,
            arrayBuffer: () => Promise.resolve(buffer),
            text: () => Promise.resolve(new TextDecoder().decode(buffer)),
            stream: () => { throw new Error('Blob.stream() is not supported'); }
        };
    }

    async text() {
        if (this.#bodyUsed) throw new TypeError('Body has already been consumed');
        this.#bodyUsed = true;

        if (this.#body === null) {
            return '';
        }

        if (typeof this.#body === 'string') {
            return this.#body;
        }

        if (this.#body instanceof ReadableStream) {
            const chunks = [];

            for await (const chunk of this.#body) {
                chunks.push(chunk);
            }

            // Concatenate all chunks
            let totalLength = 0;
            for (const chunk of chunks) {
                totalLength += chunk.length;
            }

            const combined = new Uint8Array(totalLength);
            let offset = 0;
            for (const chunk of chunks) {
                combined.set(chunk, offset);
                offset += chunk.length;
            }

            return new TextDecoder().decode(combined);
        }

        return String(this.#body);
    }

    async json() {
        const text = await this.text();
        return JSON.parse(text);
    }

    async formData() {
        throw new Error('Response.formData() is not supported');
    }

    async bytes() {
        const buffer = await this.arrayBuffer();
        return new Uint8Array(buffer);
    }
}
"#;
