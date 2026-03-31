// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Request class polyfill JavaScript

pub const JS_REQUEST: &str = r#"
// ============= Request =============
class Request {
    #url;
    #method;
    #headers;
    #body;
    #mode;
    #credentials;
    #cache;
    #redirect;
    #referrer;
    #referrerPolicy;
    #signal;
    #bodyUsed = false;

    constructor(input, init = {}) {
        if (input instanceof Request) {
            this.#url = input.url;
            this.#method = init.method ?? input.method;
            this.#headers = new Headers(init.headers ?? input.headers);
            this.#body = init.body !== undefined ? init.body : input._getBody();
            this.#mode = init.mode ?? input.mode;
            this.#credentials = init.credentials ?? input.credentials;
            this.#cache = init.cache ?? input.cache;
            this.#redirect = init.redirect ?? input.redirect;
            this.#referrer = init.referrer ?? input.referrer;
            this.#referrerPolicy = init.referrerPolicy ?? input.referrerPolicy;
            this.#signal = init.signal ?? input.signal;
        } else {
            this.#url = String(input);
            this.#method = (init.method ?? 'GET').toUpperCase();
            this.#headers = new Headers(init.headers);
            this.#body = init.body ?? null;
            this.#mode = init.mode ?? 'cors';
            this.#credentials = init.credentials ?? 'same-origin';
            this.#cache = init.cache ?? 'default';
            this.#redirect = init.redirect ?? 'follow';
            this.#referrer = init.referrer ?? 'about:client';
            this.#referrerPolicy = init.referrerPolicy ?? '';
            this.#signal = init.signal ?? null;
        }
    }

    get url() { return this.#url; }
    get method() { return this.#method; }
    get headers() { return this.#headers; }
    get mode() { return this.#mode; }
    get credentials() { return this.#credentials; }
    get cache() { return this.#cache; }
    get redirect() { return this.#redirect; }
    get referrer() { return this.#referrer; }
    get referrerPolicy() { return this.#referrerPolicy; }
    get signal() { return this.#signal; }
    get bodyUsed() { return this.#bodyUsed; }

    clone() {
        if (this.#bodyUsed) {
            throw new TypeError('Body has already been consumed');
        }
        return new Request(this);
    }

    // Internal: get body for fetch
    _getBody() { return this.#body; }

    // Body mixin methods
    async arrayBuffer() {
        if (this.#bodyUsed) throw new TypeError('Body has already been consumed');
        this.#bodyUsed = true;
        const text = await this.text();
        return new TextEncoder().encode(text).buffer;
    }

    async blob() {
        throw new Error('Request.blob() is not supported');
    }

    async formData() {
        throw new Error('Request.formData() is not supported');
    }

    async json() {
        const text = await this.text();
        return JSON.parse(text);
    }

    async text() {
        if (this.#bodyUsed) throw new TypeError('Body has already been consumed');
        this.#bodyUsed = true;
        if (this.#body === null) return '';
        if (typeof this.#body === 'string') return this.#body;
        if (this.#body instanceof FormData) {
            throw new Error('Cannot convert FormData to text');
        }
        return JSON.stringify(this.#body);
    }
}
"#;
