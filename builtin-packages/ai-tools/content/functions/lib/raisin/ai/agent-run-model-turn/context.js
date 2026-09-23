/**
 * Context construction and output budgeting for one model turn of a run.
 *
 * The context is: the agent's system prompt (+ planning, the run owner's
 * memory, skills and rules), the reducer's phase
 * instructions, the run's AUTHORITATIVE facts (plan, progress, writes) rendered
 * from structured state — never recalled from prose — and the transcript.
 *
 * Budgeting keeps a turn inside the model's window without losing the facts:
 * each tool result is capped, and when the whole context is still too large
 * the oldest exchanges are dropped as UNITS (an assistant message with its
 * tool results), so a provider never sees an orphaned tool result.
 */

import { getPlanningSystemPromptAddition } from '../agent-shared/utils.js';
import { loadUserMemory, formatMemoryForPrompt } from '../agent-shared/memory.js';
import { composeInstructionTail, appendTail } from '../agent-shared/skills.js';

export const DEFAULT_CONTEXT_TOKENS = 48000;
export const DEFAULT_TOOL_RESULT_CHARS = 12000;
const CHARS_PER_TOKEN = 4;

/** A rough, provider-neutral token estimate. */
export function estimateTokens(messages) {
  let chars = 0;
  for (const m of messages || []) {
    chars += typeof m.content === 'string' ? m.content.length : JSON.stringify(m.content || '').length;
    if (m.tool_calls) chars += JSON.stringify(m.tool_calls).length;
  }
  return Math.ceil(chars / CHARS_PER_TOKEN);
}

/** The agent's context budget in tokens. */
export function contextBudget(agentProps) {
  const explicit = Number(agentProps.max_context_tokens);
  if (explicit > 0) return Math.floor(explicit);
  const conv = Number(agentProps.max_conversation_tokens);
  if (conv > 0) return Math.max(8000, Math.floor(conv * 0.6));
  return DEFAULT_CONTEXT_TOKENS;
}

/** The run's facts as a system block. Structured, so correctness never rests on a summary. */
export function renderFacts(facts) {
  if (!facts || typeof facts !== 'object') return '';
  const lines = ['## Run state (authoritative, from the runtime)'];
  if (facts.objective) lines.push(`Objective: ${String(facts.objective).slice(0, 600)}`);
  const plan = facts.plan;
  if (plan && Array.isArray(plan.tasks)) {
    lines.push(`Plan "${plan.title}" (${plan.status}):`);
    for (const t of plan.tasks) {
      const flag = t.completion_requested && t.status !== 'completed' ? ' — completion requested, NOT granted (no evidence)' : '';
      lines.push(`- ${t.key} [${t.status}] ${t.title}${flag}`);
    }
  }
  const p = facts.progress;
  if (p) {
    lines.push(`Progress: ${p.model_turns} model turns, ${p.tool_ok} tool calls succeeded, ${p.tool_failed} failed, ${p.refused} refused, ${p.writes} writes.`);
  }
  if (Array.isArray(facts.recent_writes) && facts.recent_writes.length) {
    lines.push('Nodes this run wrote (the only paths that exist because of this run):');
    for (const w of facts.recent_writes.slice(-10)) lines.push(`- ${w.action} ${w.workspace}:${w.path}`);
  }
  if (Array.isArray(facts.diagnostics) && facts.diagnostics.length) {
    lines.push('Runtime notices:');
    for (const d of facts.diagnostics.slice(-3)) lines.push(`- ${d.code}: ${d.message}`);
  }
  return lines.length > 1 ? lines.join('\n') : '';
}

/** The system prompt of a run's model turn. */
export async function buildSystemPrompt({ agentProps, request, memoryOwner, skills, definitions }) {
  let prompt = agentProps.system_prompt || '';
  const offeredNames = new Set(definitions.map((d) => d.function.name.replace(/-/g, '_')));
  if (offeredNames.has('create_plan') || offeredNames.has('update_task')) {
    prompt += `\n${getPlanningSystemPromptAddition(agentProps.execution_mode)}`;
    prompt += '\nUnder this runtime a task is marked completed only when a tool operation succeeded while it was in progress; requesting completion without that is refused.';
  }
  if (memoryOwner) {
    const memory = await loadUserMemory(memoryOwner.agentName, memoryOwner.userId);
    if (memory) prompt += formatMemoryForPrompt(memory);
  }
  prompt = appendTail(prompt, composeInstructionTail({ skills, rules: agentProps.rules }));
  if (request.instructions) prompt += `\n\n## Instructions for this step\n${request.instructions}`;
  const facts = renderFacts(request.context && request.context.facts);
  if (facts) prompt += `\n\n${facts}`;
  if (request.output_schema) {
    prompt += `\n\nWhen you answer without a tool call, answer with ONE JSON value matching this schema:\n${JSON.stringify(request.output_schema)}`;
  }
  return prompt;
}

/**
 * Restate the step's instructions as the LATEST message when the reducer asks
 * for it (`context.restate_instructions`). The system prompt carries them
 * too, but it opens with the agent's own prompt and skills; a model that
 * answers the conversation's last message (a tool result, its own summary)
 * reads past a directive buried there — measured: a repair turn answered in
 * prose, twice, with `revise_artifact` offered and asked for. A reducer opts
 * in PER TURN, for a turn whose instructions carry something new (a repair, a
 * steer, a nudge) — restating a standing "read what exists" after every read
 * kept a model re-reading. The message is context only, never written to the
 * transcript. Not after a person's own message: that is what the turn answers.
 */
export function restateInstructions(messages, request) {
  const hints = request && request.context;
  if (!hints || hints.restate_instructions !== true || !request.instructions) return messages;
  const last = messages[messages.length - 1];
  if (last && last.role === 'user') return messages;
  return messages.concat({
    role: 'user',
    content: `[Runtime — this is the run, not the person. For this turn:]\n${request.instructions}`,
  });
}

function capToolContent(entry, maxChars) {
  if (entry.role !== 'tool' || typeof entry.content !== 'string' || entry.content.length <= maxChars) return entry;
  return {
    ...entry,
    content: `${entry.content.slice(0, maxChars)}\n[truncated: ${entry.content.length} characters, first ${maxChars} shown]`,
  };
}

/** Group history after the leading system entries into droppable units. */
function units(entries) {
  const out = [];
  for (const e of entries) {
    if (e.role === 'tool' && out.length) out[out.length - 1].push(e);
    else out.push([e]);
  }
  return out;
}

/**
 * Fit `history` into `budgetTokens`: cap tool results, then drop the oldest
 * units — never the leading system entries, never the last unit.
 */
export function applyBudget(history, { budgetTokens, maxToolChars = DEFAULT_TOOL_RESULT_CHARS }) {
  const capped = history.map((e) => capToolContent(e, maxToolChars));
  let lead = 0;
  while (lead < capped.length && capped[lead].role === 'system') lead++;
  const head = capped.slice(0, lead);
  let body = units(capped.slice(lead));
  let dropped = 0;
  while (body.length > 1 && estimateTokens(head.concat(...body)) > budgetTokens) {
    body = body.slice(1);
    dropped += 1;
  }
  // A context must not open with an orphaned tool result.
  while (body.length > 1 && body[0][0].role === 'tool') body = body.slice(1);
  const messages = head.concat(...body);
  if (dropped > 0) {
    messages.splice(lead, 0, {
      role: 'system',
      content: `[${dropped} older exchange(s) omitted to fit the context budget; the run state above is authoritative.]`,
    });
  }
  return { messages, dropped, tokens: estimateTokens(messages) };
}
