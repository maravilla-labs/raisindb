// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

/**
 * Binds the real Component Model host as a side effect of being imported.
 *
 * The specifier is the WIT interface from `wit/raisin-function.wit`
 * (`package raisin:function@0.1.0`, `interface host`) and is resolvable ONLY
 * inside `jco componentize`. Nothing else in this SDK imports this module, so
 * `vitest` never sees the unresolvable specifier.
 *
 * The VERSION SUFFIX is required: verified against componentize-js 0.22.0,
 * `'raisin:function/host'` fails at Wizer time with `Error loading module`,
 * while `'raisin:function/host@0.1.0'` links. It must track the WIT package
 * version (and therefore `SDK_ABI_VERSION`).
 *
 * jco lowers WIT kebab-case to lowerCamelCase, so `abi-version` arrives as
 * `abiVersion`, and lifts `log-level` enum values as plain strings.
 */
// eslint-disable-next-line import/no-unresolved
import { call, log, context, abiVersion } from 'raisin:function/host@0.1.0';
import { setHost } from './host.js';

setHost({ call, log, context, abiVersion });
