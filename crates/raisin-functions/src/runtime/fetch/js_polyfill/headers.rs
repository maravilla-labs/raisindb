// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! DOMException and Headers polyfill JavaScript

pub const JS_HEADERS: &str = r#"
// ============= DOMException polyfill =============
class DOMException extends Error {
    constructor(message, name = 'Error') {
        super(message);
        this.name = name;
    }
}

// ============= Headers =============
class Headers {
    #map = new Map();

    constructor(init) {
        if (init instanceof Headers) {
            init.forEach((value, key) => this.append(key, value));
        } else if (Array.isArray(init)) {
            for (const [key, value] of init) {
                this.append(key, value);
            }
        } else if (init && typeof init === 'object') {
            for (const [key, value] of Object.entries(init)) {
                this.append(key, value);
            }
        }
    }

    append(name, value) {
        const key = name.toLowerCase();
        const existing = this.#map.get(key);
        this.#map.set(key, existing ? `${existing}, ${value}` : String(value));
    }

    delete(name) {
        this.#map.delete(name.toLowerCase());
    }

    get(name) {
        return this.#map.get(name.toLowerCase()) ?? null;
    }

    has(name) {
        return this.#map.has(name.toLowerCase());
    }

    set(name, value) {
        this.#map.set(name.toLowerCase(), String(value));
    }

    forEach(callback, thisArg) {
        this.#map.forEach((value, key) => {
            callback.call(thisArg, value, key, this);
        });
    }

    *entries() { yield* this.#map.entries(); }
    *keys() { yield* this.#map.keys(); }
    *values() { yield* this.#map.values(); }
    [Symbol.iterator]() { return this.entries(); }

    // Convert to plain object for JSON serialization
    toJSON() {
        const obj = {};
        this.forEach((v, k) => obj[k] = v);
        return obj;
    }

    // Get all entries as an object (internal use)
    _toObject() {
        const obj = {};
        this.forEach((v, k) => obj[k] = v);
        return obj;
    }
}
"#;
