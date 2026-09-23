/**
 * One provider request for a run's model turn, with BOUNDED retries.
 *
 * - transient provider errors (timeouts, 429, 5xx, overload): up to
 *   `transientRetries` more attempts;
 * - a malformed tool call refused by the provider: ONE corrected retry
 *   (agent-shared/completion-retry.js);
 * - malformed output the provider returned anyway — tool calls with unusable
 *   names or arguments, raw `<function=…>` syntax, "Calling x" echoed as text,
 *   an empty answer, or JSON that does not parse when an output schema was
 *   asked for: ONE corrective retry each, then an honest fallback.
 *
 * Returns the normalized turn `{ message:{text}, tool_calls:[{call_id,name,args}],
 * finish_reason, usage:{input_tokens,output_tokens}, model, retries[] }` or
 * throws an error carrying `error_class` and `retryable`.
 */

import {
  normalizeCompletionResponse, normalizeToolCalls, parseToolArguments, getToolCallName, stripRuntimeArgs,
} from '../agent-shared/tools.js';
import { completeWithToolCallRetry, malformedToolCallCorrection, MALFORMED_TOOL_CALL_CODE } from '../agent-shared/completion-retry.js';
import { classifyError, statusForClass } from '../agent-shared/tool-envelope.js';
import { TERMINAL_FALLBACK_TEXT } from '../agent-shared/utils.js';

const RAW_FUNCTION = /<function=[\w.:-]+>/;

function classified(err, cls, retryable) {
  const out = err instanceof Error ? err : new Error(String(err));
  out.error_class = cls;
  out.retryable = retryable;
  return out;
}

/** Call `fn`, retrying transient failures up to `retries` times. */
export async function withTransientRetry(fn, retries, onRetry) {
  let attempt = 0;
  for (;;) {
    try {
      return await fn();
    } catch (err) {
      const cls = classifyError(err);
      if (statusForClass(cls) !== 'retryable' || attempt >= retries || err.code === MALFORMED_TOOL_CALL_CODE) {
        throw classified(err, cls === 'tool_error' ? 'provider' : cls, statusForClass(cls) === 'retryable');
      }
      attempt += 1;
      if (onRetry) onRetry(cls, attempt, err);
    }
  }
}

/** Provider tool calls → `{call_id, name, args}`; unusable ones reported apart. */
export function extractCalls(rawCalls, operationId) {
  const { normalized, malformed } = normalizeToolCalls(rawCalls);
  const calls = [];
  const bad = [...malformed];
  const seen = new Set();
  normalized.forEach((tc, i) => {
    let args;
    try {
      args = stripRuntimeArgs(parseToolArguments(tc));
    } catch (_) {
      bad.push(tc);
      return;
    }
    if (!args || typeof args !== 'object' || Array.isArray(args)) args = {};
    delete args.__raisin_context;
    let id = String(tc.id || '').replace(/\0/g, '').slice(0, 200) || `${operationId}#${i}`;
    if (seen.has(id)) id = `${id}#${i}`;
    seen.add(id);
    calls.push({ call_id: id, name: getToolCallName(tc), args });
  });
  return { calls, bad };
}

function usageOf(raw) {
  const u = (raw && raw.usage) || {};
  return {
    input_tokens: Number(u.prompt_tokens ?? u.input_tokens ?? 0) || 0,
    output_tokens: Number(u.completion_tokens ?? u.output_tokens ?? 0) || 0,
  };
}

function addUsage(a, b) {
  return { input_tokens: a.input_tokens + b.input_tokens, output_tokens: a.output_tokens + b.output_tokens };
}

function parsesAsJson(text) {
  try {
    JSON.parse(String(text || '').trim().replace(/^```(?:json)?\s*|\s*```$/g, ''));
    return true;
  } catch (_) {
    return false;
  }
}

/**
 * Run the turn. `complete(messages, {tools, stream})` performs one provider
 * call (raisin.ai.completion bound to model, channel and limits).
 */
export async function runModelTurn({ complete, messages, tools, operationId, outputSchema, transientRetries = 2 }) {
  const retries = [];
  const hasTools = Array.isArray(tools) && tools.length > 0;
  let usage = { input_tokens: 0, output_tokens: 0 };
  const call = (msgs, opts = {}) => withTransientRetry(
    () => complete(msgs, { tools: opts.noTools ? undefined : (hasTools ? tools : undefined), stream: opts.stream !== false }),
    transientRetries,
    (cls, n) => retries.push({ kind: 'transient', class: cls, attempt: n }),
  );

  const first = hasTools
    ? await completeWithToolCallRetry((m) => call(m), messages, () => retries.push({ kind: 'malformed_tool_call' }))
      .catch((err) => {
        if (err && err.code === MALFORMED_TOOL_CALL_CODE) throw classified(err, 'malformed_output', true);
        throw err;
      })
    : { raw: await call(messages) };
  let raw = first.raw;
  usage = addUsage(usage, usageOf(raw));
  let resp = normalizeCompletionResponse(raw);
  let { calls, bad } = extractCalls(resp.tool_calls, operationId);
  let text = resp.content || '';

  const retryWith = async (extra, kind, opts = {}) => {
    retries.push({ kind });
    const again = await call([...messages, ...extra], { stream: false, ...opts });
    usage = addUsage(usage, usageOf(again));
    const r = normalizeCompletionResponse(again);
    const ex = extractCalls(r.tool_calls, operationId);
    return { r, ...ex };
  };

  if (hasTools && bad.length > 0 && calls.length === 0 && !text.trim()) {
    const a = await retryWith([malformedToolCallCorrection(JSON.stringify(bad).slice(0, 2000))], 'malformed_tool_calls');
    if (a.calls.length || a.r.content.trim()) ({ r: resp, calls, bad } = a), (text = a.r.content || '');
  }
  if (calls.length === 0 && RAW_FUNCTION.test(text)) {
    const a = await retryWith([
      { role: 'assistant', content: text },
      { role: 'user', content: 'Your previous response used invalid function call syntax. Use the tool-calling mechanism, or respond in plain text.' },
    ], 'raw_function_syntax');
    if (a.calls.length || (a.r.content.trim() && !RAW_FUNCTION.test(a.r.content))) ({ r: resp, calls } = a), (text = a.r.content || '');
    text = text.replace(/<function=[\w.:-]+>[\s\S]*?<\/function>/g, '').trim();
  }
  if (hasTools && calls.length === 0 && /^Calling\s+[\w-]+/i.test(text.trim())) {
    const a = await retryWith([
      { role: 'assistant', content: text },
      { role: 'system', content: 'You wrote a tool name as plain text instead of calling it. Use the function-calling mechanism to invoke the tool now.' },
    ], 'tool_echo');
    if (a.calls.length || a.r.content.trim()) ({ r: resp, calls } = a), (text = a.r.content || '');
  }
  if (calls.length === 0 && !text.trim()) {
    const a = await retryWith([
      { role: 'system', content: 'You returned an empty response. Respond now: call a tool, or answer the user in plain text.' },
    ], 'empty_response');
    if (a.calls.length || a.r.content.trim()) ({ r: resp, calls } = a), (text = a.r.content || '');
  }
  if (outputSchema && calls.length === 0 && text.trim() && !parsesAsJson(text)) {
    const a = await retryWith([
      { role: 'assistant', content: text },
      { role: 'system', content: `Answer with ONE JSON value matching this schema and nothing else:\n${JSON.stringify(outputSchema)}` },
    ], 'output_schema', { noTools: true });
    if (a.r.content.trim() && parsesAsJson(a.r.content)) text = a.r.content;
  }
  if (calls.length === 0 && !text.trim()) text = TERMINAL_FALLBACK_TEXT;

  return {
    message: { text },
    tool_calls: calls,
    finish_reason: calls.length ? 'tool_calls' : (resp.finish_reason || 'stop'),
    usage,
    model: resp.model || null,
    thinking: Array.isArray(resp.thinking) ? resp.thinking : [],
    retries,
  };
}
