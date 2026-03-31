// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! JavaScript polyfill for setTimeout/setInterval APIs
//!
//! This provides the standard Web API timer functions that wrap the internal
//! Rust implementation exposed via `__timers_internal`.

/// JavaScript polyfill code for timer APIs
///
/// This code is evaluated during runtime initialization to provide:
/// - `setTimeout(callback, delay, ...args)` - Returns timer ID
/// - `clearTimeout(timerId)` - Cancels a pending timeout
/// - `setInterval(callback, interval, ...args)` - Returns timer ID
/// - `clearInterval(timerId)` - Cancels a repeating interval
///
/// The implementation stores callbacks in a Map and uses Rust-side promises
/// to handle the actual timing and cancellation.
pub const TIMERS_POLYFILL: &str = r#"
(function() {
    'use strict';

    // Storage for pending callbacks
    const pendingCallbacks = new Map();

    /**
     * Schedule a callback to run after a delay
     * @param {Function} callback - Function to call
     * @param {number} delay - Delay in milliseconds (default: 0)
     * @param {...any} args - Arguments to pass to callback
     * @returns {string} Timer ID for cancellation
     */
    globalThis.setTimeout = function(callback, delay, ...args) {
        if (typeof callback !== 'function') {
            throw new TypeError('Callback must be a function');
        }

        const delayMs = Math.max(0, parseInt(delay) || 0);

        // Get timer ID and create the delay promise
        const timerId = __timers_internal.create_timer(delayMs);

        // Store callback info
        pendingCallbacks.set(timerId, { callback, args, cancelled: false });

        // Start the async wait - when it resolves, execute callback
        __timers_internal.wait_timer(timerId, delayMs).then(completed => {
            const entry = pendingCallbacks.get(timerId);
            pendingCallbacks.delete(timerId);

            // Only execute if timer completed (not cancelled)
            if (completed && entry && !entry.cancelled) {
                try {
                    entry.callback(...entry.args);
                } catch (e) {
                    console.error('setTimeout callback error:', e);
                }
            }
        }).catch(err => {
            // Timer was cancelled or errored
            pendingCallbacks.delete(timerId);
        });

        return timerId;
    };

    /**
     * Cancel a pending timeout
     * @param {string} timerId - Timer ID from setTimeout
     */
    globalThis.clearTimeout = function(timerId) {
        if (timerId == null) return;

        const entry = pendingCallbacks.get(timerId);
        if (entry) {
            entry.cancelled = true;
        }

        __timers_internal.cancel_timer(String(timerId));
        pendingCallbacks.delete(timerId);
    };

    /**
     * Schedule a callback to run repeatedly at an interval
     * @param {Function} callback - Function to call
     * @param {number} interval - Interval in milliseconds
     * @param {...any} args - Arguments to pass to callback
     * @returns {string} Timer ID for cancellation
     */
    globalThis.setInterval = function(callback, interval, ...args) {
        if (typeof callback !== 'function') {
            throw new TypeError('Callback must be a function');
        }

        const intervalMs = Math.max(1, parseInt(interval) || 0);

        // Create a wrapper that reschedules itself
        let currentTimerId = null;
        let cancelled = false;

        const scheduleNext = () => {
            if (cancelled) return;

            currentTimerId = __timers_internal.create_timer(intervalMs);
            pendingCallbacks.set(currentTimerId, { isInterval: true, cancel: () => { cancelled = true; } });

            __timers_internal.wait_timer(currentTimerId, intervalMs).then(completed => {
                pendingCallbacks.delete(currentTimerId);

                if (completed && !cancelled) {
                    try {
                        callback(...args);
                    } catch (e) {
                        console.error('setInterval callback error:', e);
                    }
                    // Schedule next iteration
                    scheduleNext();
                }
            }).catch(() => {
                pendingCallbacks.delete(currentTimerId);
            });
        };

        // Start the interval
        scheduleNext();

        // Return the first timer ID (clearInterval will mark cancelled)
        // Store a reference to cancel function
        const intervalId = `interval_${currentTimerId}`;
        pendingCallbacks.set(intervalId, {
            isInterval: true,
            cancel: () => {
                cancelled = true;
                if (currentTimerId) {
                    __timers_internal.cancel_timer(String(currentTimerId));
                }
            }
        });

        return intervalId;
    };

    /**
     * Cancel a repeating interval
     * @param {string} intervalId - Timer ID from setInterval
     */
    globalThis.clearInterval = function(intervalId) {
        if (intervalId == null) return;

        const entry = pendingCallbacks.get(intervalId);
        if (entry && entry.cancel) {
            entry.cancel();
        }
        pendingCallbacks.delete(intervalId);

        // Also try to cancel as regular timer (in case someone passes setTimeout ID)
        __timers_internal.cancel_timer(String(intervalId));
    };

})();
"#;
