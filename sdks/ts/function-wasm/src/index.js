// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// Public entry point of `@raisindb/function-wasm`.

export { createHandler, resolveHandler, registeredHandlers } from './entry.js';
export { installShim } from './shim.js';
export { setHost, clearHost, host, hasHost, SDK_ABI_VERSION } from './host.js';
