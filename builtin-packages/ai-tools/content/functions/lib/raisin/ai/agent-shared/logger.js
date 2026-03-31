/**
 * Structured logging with SSE event emission for agent handlers.
 *
 * Logs are both printed to console and forwarded to the conversation SSE
 * stream via raisin.events.emit('conversation:log', ...) so the frontend
 * can display them in the browser console.
 */

let _context = {};

/**
 * Set global context fields that are appended to every log line.
 * Typically called once per handler invocation with { chat, channel }.
 */
export function setContext(ctx) {
  _context = { ...ctx };
}

/** Format a structured log line: [module] message | key=value pairs */
function fmt(module, message, extra) {
  const parts = [`[${module}]`, message];
  const merged = { ..._context, ...extra };
  const kvPairs = Object.entries(merged)
    .filter(([, v]) => v !== undefined && v !== null)
    .map(([k, v]) => `${k}=${typeof v === 'object' ? JSON.stringify(v) : v}`)
    .join(' ');
  if (kvPairs) parts.push('|', kvPairs);
  return parts.join(' ');
}

/** Emit a log event to the conversation SSE stream. */
function emitLogEvent(level, module, formattedMessage) {
  const channel = _context.channel;
  if (!channel) return;
  if (typeof globalThis.raisin === 'undefined') return;

  try {
    raisin.events.emit('conversation:log', {
      type: 'log',
      level,
      message: formattedMessage,
      module: module || undefined,
      channel,
      conversationPath: _context.chat || undefined,
      timestamp: new Date().toISOString(),
    });
  } catch (_) {
    // Never let log emission break handler flow
  }
}

export const log = {
  info(mod, msg, extra) {
    const m = fmt(mod, msg, extra);
    console.info(m);
    emitLogEvent('info', mod, m);
  },
  debug(mod, msg, extra) {
    const m = fmt(mod, msg, extra);
    console.debug(m);
    emitLogEvent('debug', mod, m);
  },
  warn(mod, msg, extra) {
    const m = fmt(mod, msg, extra);
    console.warn(m);
    emitLogEvent('warn', mod, m);
  },
  error(mod, msg, extra) {
    const m = fmt(mod, msg, extra);
    console.error(m);
    emitLogEvent('error', mod, m);
  },
  /** Numbered step log: STEP n/total: message */
  step(mod, n, total, msg, extra) {
    const m = fmt(mod, `STEP ${n}/${total}: ${msg}`, extra);
    console.info(m);
    emitLogEvent('info', mod, m);
  },
  /** Start a timer — returns a timestamp for use with since(). */
  time() {
    return Date.now();
  },
  /** Milliseconds elapsed since a time() call. */
  since(t0) {
    return Date.now() - t0;
  },
};
