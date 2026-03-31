// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AbortSignal and AbortController polyfill JavaScript

pub const JS_ABORT: &str = r#"
// ============= AbortSignal =============
class AbortSignal {
    #id;
    #aborted = false;
    #reason = undefined;
    #listeners = [];

    static abort(reason) {
        const controller = new AbortController();
        controller.abort(reason);
        return controller.signal;
    }

    static timeout(ms) {
        const controller = new AbortController();
        setTimeout(() => controller.abort(new DOMException('The operation timed out.', 'TimeoutError')), ms);
        return controller.signal;
    }

    constructor(id) {
        this.#id = id;
    }

    get aborted() {
        // Poll Rust for current state if we have an ID
        if (!this.#aborted && this.#id) {
            this.#aborted = __raisin_internal.fetch_is_aborted(this.#id);
        }
        return this.#aborted;
    }

    get reason() { return this.#reason; }

    // Internal: Get the controller ID
    get _id() { return this.#id; }

    // Internal: Set aborted state (called by AbortController)
    _setAborted(reason) {
        if (this.#aborted) return;
        this.#aborted = true;
        this.#reason = reason;
        // Trigger abort event listeners
        for (const listener of this.#listeners) {
            try { listener.call(this, { type: 'abort', target: this }); } catch (e) {}
        }
    }

    addEventListener(type, listener) {
        if (type === 'abort') {
            this.#listeners.push(listener);
        }
    }

    removeEventListener(type, listener) {
        if (type === 'abort') {
            const idx = this.#listeners.indexOf(listener);
            if (idx >= 0) this.#listeners.splice(idx, 1);
        }
    }

    throwIfAborted() {
        if (this.aborted) {
            throw this.reason ?? new DOMException('The operation was aborted.', 'AbortError');
        }
    }
}

// ============= AbortController =============
class AbortController {
    #signal;
    #id;

    constructor() {
        this.#id = __raisin_internal.fetch_create_abort_controller();
        this.#signal = new AbortSignal(this.#id);
    }

    get signal() { return this.#signal; }

    abort(reason) {
        const abortReason = reason ?? new DOMException('The operation was aborted.', 'AbortError');
        __raisin_internal.fetch_abort(this.#id, abortReason?.message ?? String(abortReason));
        this.#signal._setAborted(abortReason);
    }
}
"#;
