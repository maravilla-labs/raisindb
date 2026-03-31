// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! ReadableStream and ReadableStreamDefaultReader polyfill JavaScript

pub const JS_STREAMS: &str = r#"
// ============= ReadableStream (Simplified) =============
class ReadableStream {
    #streamId;
    #locked = false;

    constructor(streamIdOrSource) {
        if (typeof streamIdOrSource === 'string') {
            // Internal: created with stream ID from Rust
            this.#streamId = streamIdOrSource;
        } else {
            // User-created ReadableStream - not fully supported
            throw new Error('Creating custom ReadableStreams is not supported');
        }
    }

    get locked() { return this.#locked; }

    // Internal: get stream ID
    get _streamId() { return this.#streamId; }

    getReader(options) {
        if (this.#locked) {
            throw new TypeError('ReadableStream is locked');
        }
        this.#locked = true;
        __raisin_internal.fetch_stream_lock(this.#streamId);
        return new ReadableStreamDefaultReader(this.#streamId, this);
    }

    async cancel(reason) {
        __raisin_internal.fetch_stream_cancel(this.#streamId);
    }

    // Async iteration support
    async *[Symbol.asyncIterator]() {
        const reader = this.getReader();
        try {
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;
                yield value;
            }
        } finally {
            reader.releaseLock();
        }
    }

    tee() {
        throw new Error('ReadableStream.tee() is not supported');
    }

    pipeTo(dest, options) {
        throw new Error('ReadableStream.pipeTo() is not supported');
    }

    pipeThrough(transform, options) {
        throw new Error('ReadableStream.pipeThrough() is not supported');
    }
}

class ReadableStreamDefaultReader {
    #streamId;
    #stream;
    #released = false;

    constructor(streamId, stream) {
        this.#streamId = streamId;
        this.#stream = stream;
    }

    async read() {
        if (this.#released) {
            throw new TypeError('Reader has been released');
        }

        // Call Rust to read next chunk
        const resultJson = __raisin_internal.fetch_stream_read(this.#streamId);
        const result = JSON.parse(resultJson);

        if (result.status === 'error') {
            if (result.message === 'PENDING') {
                // No data yet, wait and retry
                await new Promise(resolve => setTimeout(resolve, 1));
                return this.read();
            }
            throw new Error(result.message);
        }

        if (result.status === 'done') {
            return { done: true, value: undefined };
        }

        // Decode base64 to Uint8Array
        const binary = atob(result.value);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) {
            bytes[i] = binary.charCodeAt(i);
        }

        return { done: false, value: bytes };
    }

    releaseLock() {
        if (!this.#released) {
            this.#released = true;
            __raisin_internal.fetch_stream_unlock(this.#streamId);
            // Note: We can't easily unlock the stream reference here
            // The stream will be unlocked when cleaned up
        }
    }

    get closed() {
        // Return a promise that resolves when the stream is done
        // This is a simplified implementation
        return Promise.resolve();
    }

    async cancel(reason) {
        __raisin_internal.fetch_stream_cancel(this.#streamId);
    }
}
"#;
