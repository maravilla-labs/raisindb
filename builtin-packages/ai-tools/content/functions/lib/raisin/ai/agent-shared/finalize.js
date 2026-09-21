/**
 * THE FINALIZE GATE — shared by BOTH terminal paths.
 *
 * It lives in `agent-shared/` because a gate applied on one of the two paths is
 * not a gate, and both handlers must reach the SAME code. It used to live in
 * `agent-handler/index.js`, which `agent-continue-handler` imported directly;
 * the runtime refuses that import ("Error resolving module
 * '../agent-handler/index.js' from 'entry'") because a function's ENTRY file is
 * not a resolvable module for another function — only files under a shared
 * sibling directory are. The symptom was total: every multi-tool agent turn
 * stopped dead after its first tool result, with no error anywhere a user
 * could see.
 */

import { log } from './logger.js';

/* ────────────────────────────────────────────────────────────────────────────
 * THE FINALIZE GATE
 *
 * A turn ending is not a run completing, and an agent marking its own TODO
 * complete is not verification. The run loop used to take the model's word for
 * both: `update-task` wrote whatever status it was handed, and the terminal
 * path handed `response.content` to the outbox verbatim — so a stopped turn
 * could sit beside "validated and enabled" with nothing in the system
 * disagreeing.
 *
 * The gate is per AGENT, declared by `finalize_policy` on the raisin:AIAgent
 * node, for the same reason the round budget is: a builder that persists
 * executable artifacts needs evidence, and an ordinary chat agent must be
 * completely unaffected. An agent with no policy takes exactly the old path.
 *
 * EVIDENCE lives on the raisin:AITask, in the readonly `verification_ref` /
 * `verification_hash` / `proof_level` family — beside `delegation_*`, and
 * written the same way: by the server operation that obtained the proof, never
 * by the model. The gate READS it; it never mints it. Nothing here re-derives
 * proof, so the check costs one query.
 *
 * These helpers live in this file rather than in `agent-shared/` only because
 * the continuation handler already imports one-hop siblings and this file is
 * the one both terminal paths can reach without a cycle.
 * ──────────────────────────────────────────────────────────────────────────── */

/** The one policy value that turns the gate on. */
export const FINALIZE_POLICY_VERIFIED = 'require_verified_completion';

/** Proof is a LADDER, not a boolean — these are ordered weakest to strongest. */
export const PROOF_LEVELS = ['draft_validated', 'fixture_tested', 'activated'];

/** Statuses a task cannot come back from. `failed` is terminal too. */
export const TASK_TERMINAL_STATUSES = ['completed', 'cancelled', 'failed'];

/** A reference envelope, a bare path, or nothing. */
export function refToPath(ref) {
  if (!ref) return null;
  if (typeof ref === 'string') return ref;
  return ref['raisin:path'] || ref['raisin:ref'] || null;
}

export function finalizePolicyOf(agentProps) {
  const p = agentProps && agentProps.finalize_policy;
  return typeof p === 'string' && p ? p : 'none';
}

/**
 * What this task claims to have BUILT, if anything. A task with no build
 * target is ordinary work (research, a question, a decision) and is not gated:
 * there is nothing for a verification record to be about.
 */
export function taskBuildTarget(props) {
  if (!props) return null;
  return props.build_target_path || refToPath(props.build_target_ref) || null;
}

/**
 * The verification record, or null. A record needs all three parts: what was
 * verified, the revision/hash it was verified AT (so a later edit invalidates
 * it by mismatch rather than by trust), and how strong the proof is.
 */
export function taskVerification(props) {
  if (!props) return null;
  const ref = refToPath(props.verification_ref);
  const level = props.proof_level;
  if (!ref) return null;
  if (!PROOF_LEVELS.includes(level)) return null;
  return { ref, hash: props.verification_hash || null, level };
}

/**
 * Why this task may not be called completed, as a list of what is MISSING.
 * Empty list means the evidence is there. Never throws: the caller turns this
 * into a tool result the model can act on.
 */
export function verificationGaps(taskProps, buildTarget) {
  const missing = [];
  const ref = refToPath(taskProps && taskProps.verification_ref);
  const level = taskProps && taskProps.proof_level;
  if (!ref) missing.push('verification_ref');
  if (!PROOF_LEVELS.includes(level)) {
    missing.push(`proof_level (one of ${PROOF_LEVELS.join(', ')})`);
  }
  if (!(taskProps && taskProps.verification_hash)) missing.push('verification_hash');
  if (ref && buildTarget && ref !== buildTarget) {
    missing.push(`verification_ref must point at ${buildTarget} (it points at ${ref})`);
  }
  return missing;
}

/**
 * Read the agent behind a conversation, exactly the way the round budget does
 * (agent_ref on the chat node, workspace from the envelope, default
 * 'functions'). An unreadable agent yields no policy — a gate we cannot read
 * is not a licence to invent one, and the caller then behaves as before.
 */
export async function readChatAgentProps(workspace, chatPath, chatNode) {
  try {
    const chat = chatNode || (await raisin.nodes.get(workspace, chatPath));
    const ref = chat && chat.properties && chat.properties.agent_ref;
    if (!ref) return null;
    const aPath = typeof ref === 'string' ? ref : ref['raisin:path'];
    const aWs = typeof ref === 'object' ? (ref['raisin:workspace'] || 'functions') : 'functions';
    if (!aPath) return null;
    const aNode = await raisin.nodes.get(aWs, aPath);
    return (aNode && aNode.properties) || null;
  } catch (e) {
    return null;
  }
}

/**
 * Every task of this conversation, split into what is still open and what
 * claims completion without evidence. One query, no cast — casting the
 * predicate would send this to a full type scan of the `ai` workspace.
 */
export async function collectRunEvidence(workspace, chatPath) {
  let rows = [];
  try {
    rows = await raisin.sql.query(
      `SELECT path, properties FROM "${workspace}"
        WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AITask'`,
      [chatPath],
    );
  } catch (e) {
    return { tasks: [], open: [], unverified: [], strongestProof: null, readable: false };
  }
  const tasks = (rows || []).map((r) => ({ path: r.path, props: r.properties || {} }));
  const open = [];
  const unverified = [];
  let strongest = -1;
  for (const t of tasks) {
    const status = t.props.status || 'pending';
    if (!TASK_TERMINAL_STATUSES.includes(status)) {
      open.push({ path: t.path, title: t.props.title || t.path, status });
      continue;
    }
    if (status !== 'completed') continue;
    const target = taskBuildTarget(t.props);
    if (!target) continue;
    const v = taskVerification(t.props);
    if (!v || verificationGaps(t.props, target).length > 0) {
      unverified.push({ path: t.path, title: t.props.title || t.path, target });
      continue;
    }
    const idx = PROOF_LEVELS.indexOf(v.level);
    if (idx > strongest) strongest = idx;
  }
  return {
    tasks,
    open,
    unverified,
    strongestProof: strongest >= 0 ? PROOF_LEVELS[strongest] : null,
    readable: true,
  };
}

/**
 * THE SERVER'S OWN SENTENCE about this run. Derived from persisted task state
 * and verification records only — never from the model's prose, which is the
 * thing being checked.
 */
export function serverStatusStatement(evidence) {
  /* A CHECK THAT COULD NOT RUN IS NOT A PASS. `collectRunEvidence` answers
   * `readable: false` when the task query threw, and returning no statement
   * there would hand the model's prose through unaltered — the one path the
   * gate exists to close. An unreadable run is UNVERIFIED. */
  if (evidence.readable === false) {
    return {
      gated: true,
      statement:
        'Status (server-verified): UNVERIFIED — this run\'s task state could not be read, so nothing about ' +
        'it can be confirmed. It is not validated and not enabled.',
    };
  }
  if (evidence.open.length > 0) {
    const names = evidence.open.map((t) => `"${t.title}" (${t.status})`).join(', ');
    return {
      gated: true,
      statement:
        `Status (server-verified): STOPPED with ${evidence.open.length} task(s) still open — ${names}. ` +
        'Nothing here is finished, validated or enabled.',
    };
  }
  if (evidence.unverified.length > 0) {
    const names = evidence.unverified.map((t) => `"${t.title}" → ${t.target}`).join(', ');
    return {
      gated: true,
      statement:
        `Status (server-verified): UNVERIFIED — no verification record for ${names}. ` +
        'The build was authored but no proof was obtained; it is not validated and not enabled.',
    };
  }
  if (evidence.strongestProof === 'activated') {
    return { gated: false, statement: 'Status (server-verified): validated, fixture-tested and active.' };
  }
  if (evidence.strongestProof === 'fixture_tested') {
    return { gated: false, statement: 'Status (server-verified): draft validated and fixture-tested; not enabled.' };
  }
  if (evidence.strongestProof === 'draft_validated') {
    return { gated: false, statement: 'Status (server-verified): draft validated; execution untested.' };
  }
  return { gated: false, statement: null };
}

/**
 * What a terminal turn is allowed to SAY.
 *
 * When the run is gated the model's prose is REPLACED, not decorated: a
 * readiness claim cannot be reliably edited out of free text, and the prose is
 * still on the raisin:Message node for anyone reading the conversation. When
 * the run is clean the statement is prepended, so the status is stated by the
 * server either way. An agent without the policy gets its content back
 * untouched and pays for one nothing.
 */
/**
 * PERSIST THE GATED TEXT ONTO THE MESSAGE THAT WAS ALREADY WRITTEN.
 *
 * The `raisin:Message` is created before the turn is known to be terminal
 * (`isTerminal` needs the tool-call count, which needs the persisted calls), so
 * the row lands carrying the model's raw prose. Gating only the outbox copy and
 * the SSE payload left the STORED turn ungated: the stream said "STOPPED with 1
 * task still open" and a reload of the same conversation read "validated and
 * enabled and ready to use". That is the screenshot in SENOL_FIX_THIS.md, and a
 * reader who arrives late sees only the stored copy.
 *
 * So the stored body is rewritten from the same gate result the outbox and the
 * SSE use — one value, three places, no way for them to disagree. The model's own
 * prose is kept under `model_prose` rather than discarded, because the gate
 * replaces a readiness claim it cannot reliably edit, and the turn still has to
 * be inspectable afterwards.
 *
 * Failure here is logged and swallowed: an un-rewritten message is the old
 * behaviour, and throwing would lose the turn's side effects entirely.
 */
export async function persistGatedContent(workspace, messagePath, gate, originalContent) {
  if (!gate || !gate.statement || gate.content === originalContent) return false;
  try {
    await raisin.nodes.update(workspace, messagePath, {
      properties: {
        content: gate.content,
        body: { content: gate.content, message_text: gate.content },
        model_prose: originalContent,
        terminal_gated: gate.gated === true,
        terminal_gate_statement: gate.statement,
      },
    });
    return true;
  } catch (e) {
    log.warn('agent-handler', 'Failed to persist gated terminal content', {
      path: messagePath,
      error: e.message,
    });
    return false;
  }
}

export async function gateTerminalContent(workspace, chatPath, agentProps, content) {
  if (finalizePolicyOf(agentProps) !== FINALIZE_POLICY_VERIFIED) {
    return { content, gated: false, evidence: null, statement: null };
  }
  const evidence = await collectRunEvidence(workspace, chatPath);
  const { gated, statement } = serverStatusStatement(evidence);
  if (!statement) return { content, gated: false, evidence, statement: null };
  if (gated) {
    return {
      content: `${statement}\n\nThe agent's own summary of this turn is in the conversation; it is not a statement of readiness.`,
      gated: true,
      evidence,
      statement,
    };
  }
  return { content: `${statement}\n\n${content}`, gated: false, evidence, statement };
}
