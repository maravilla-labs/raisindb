// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! TextDecoder, TextEncoder, atob/btoa polyfill JavaScript

pub const JS_ENCODING: &str = r#"
// ============= TextDecoder (simplified) =============
if (typeof TextDecoder === 'undefined') {
    globalThis.TextDecoder = class TextDecoder {
        constructor(encoding = 'utf-8') {
            this.encoding = encoding.toLowerCase();
        }

        decode(input) {
            if (!input) return '';
            const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
            // Simple UTF-8 decoder
            let result = '';
            let i = 0;
            while (i < bytes.length) {
                const byte = bytes[i];
                if (byte < 0x80) {
                    result += String.fromCharCode(byte);
                    i++;
                } else if ((byte & 0xe0) === 0xc0) {
                    result += String.fromCharCode(((byte & 0x1f) << 6) | (bytes[i + 1] & 0x3f));
                    i += 2;
                } else if ((byte & 0xf0) === 0xe0) {
                    result += String.fromCharCode(
                        ((byte & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f)
                    );
                    i += 3;
                } else if ((byte & 0xf8) === 0xf0) {
                    const codePoint = ((byte & 0x07) << 18) | ((bytes[i + 1] & 0x3f) << 12) |
                        ((bytes[i + 2] & 0x3f) << 6) | (bytes[i + 3] & 0x3f);
                    // Convert to surrogate pair
                    result += String.fromCodePoint(codePoint);
                    i += 4;
                } else {
                    result += '\ufffd';
                    i++;
                }
            }
            return result;
        }
    };
}

// ============= TextEncoder (simplified) =============
if (typeof TextEncoder === 'undefined') {
    globalThis.TextEncoder = class TextEncoder {
        constructor() {
            this.encoding = 'utf-8';
        }

        encode(str) {
            const bytes = [];
            for (let i = 0; i < str.length; i++) {
                let codePoint = str.codePointAt(i);
                if (codePoint > 0xffff) i++; // Skip low surrogate

                if (codePoint < 0x80) {
                    bytes.push(codePoint);
                } else if (codePoint < 0x800) {
                    bytes.push(0xc0 | (codePoint >> 6), 0x80 | (codePoint & 0x3f));
                } else if (codePoint < 0x10000) {
                    bytes.push(
                        0xe0 | (codePoint >> 12),
                        0x80 | ((codePoint >> 6) & 0x3f),
                        0x80 | (codePoint & 0x3f)
                    );
                } else {
                    bytes.push(
                        0xf0 | (codePoint >> 18),
                        0x80 | ((codePoint >> 12) & 0x3f),
                        0x80 | ((codePoint >> 6) & 0x3f),
                        0x80 | (codePoint & 0x3f)
                    );
                }
            }
            return new Uint8Array(bytes);
        }
    };
}

// ============= atob/btoa polyfill =============
if (typeof atob === 'undefined') {
    const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=';

    globalThis.atob = function(str) {
        // Remove padding and whitespace
        str = str.replace(/[\s=]+$/g, '');
        let output = '';
        for (let i = 0; i < str.length; i += 4) {
            const a = chars.indexOf(str[i]);
            const b = chars.indexOf(str[i + 1]);
            // Handle missing characters at end of string (after padding stripped)
            const c_char = str[i + 2];
            const d_char = str[i + 3];
            const c = c_char !== undefined ? chars.indexOf(c_char) : 64;
            const d = d_char !== undefined ? chars.indexOf(d_char) : 64;

            // When c or d is 64 (padding position), treat as 0 in bit calculation
            const bits = (a << 18) | (b << 12) | ((c === 64 ? 0 : c) << 6) | (d === 64 ? 0 : d);
            output += String.fromCharCode((bits >> 16) & 0xff);
            if (c !== 64) output += String.fromCharCode((bits >> 8) & 0xff);
            if (d !== 64) output += String.fromCharCode(bits & 0xff);
        }
        return output;
    };

    globalThis.btoa = function(str) {
        let output = '';
        for (let i = 0; i < str.length; i += 3) {
            const a = str.charCodeAt(i);
            const b = str.charCodeAt(i + 1);
            const c = str.charCodeAt(i + 2);

            const bits = (a << 16) | ((b || 0) << 8) | (c || 0);
            output += chars[(bits >> 18) & 0x3f];
            output += chars[(bits >> 12) & 0x3f];
            output += i + 1 < str.length ? chars[(bits >> 6) & 0x3f] : '=';
            output += i + 2 < str.length ? chars[bits & 0x3f] : '=';
        }
        return output;
    };
}
"#;
