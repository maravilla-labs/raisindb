// RAISINDB_GATE_PROBE_MARKER deploy-route proof 2026-09-20
/**
 * update-task — Updates a task's status and propagates progress to the parent plan.
 *
 * Status transitions: pending -> in_progress -> completed | cancelled
 * When all tasks complete, the parent plan is marked completed.
 *
 * Also updates any ai_plan message cards in the conversation so the frontend
 * plan projection stays in sync.
 *
 * Execution mode: inline
 * Category: planning
 */

import {
  FINALIZE_POLICY_VERIFIED,
  PROOF_LEVELS,
  finalizePolicyOf,
  readChatAgentProps,
  taskBuildTarget,
  verificationGaps,
} from '../agent-shared/finalize.js';
import {
  artifactOf,
  collectRunWrites,
  describeArtifact,
  evidenceFields,
  findEvidence,
  inFlightSiblings,
  sameArtifact,
  staleningWrite,
  supportingEvidence,
  supportingEvidenceFields,
  supportingHint,
  supportingKind,
  writesForTask,
} from '../agent-shared/run-evidence.js';
import { updatePlanProgress } from './plan-progress.js';

const VALID_STATUSES = ['pending', 'in_progress', 'completed', 'cancelled', 'failed'];

/* ── THE FINALIZE GATE ─────────────────────────────────────────────────────
 *
 * This tool's only guard used to be the monotonic downgrade check below; the
 * status the model asked for was then written straight onto the node with no
 * read of any artifact, compile verdict or evidence record. So "completed" meant
 * the model said so, and a plan flipped to completed by counting the boxes the
 * model had ticked itself.
 *
 * The gate is per AGENT — `finalize_policy: require_verified_completion` on the
 * raisin:AIAgent node — for the same reason the round budget is per agent: a
 * builder that persists executable artifacts needs evidence, and an ordinary
 * chat agent must be completely unaffected. Two cheap reads, exactly as
 * agent-continue-handler reads its budget.
 *
 * It gates a BUILD task: one that declares a build target, OR one that wrote an
 * artifact in this run. The second half is derived server-side from the run's
 * own persisted tool calls (`agent-shared/run-evidence.js`), because a gate
 * keyed only on the declaration was bypassed by not declaring: measured
 * 2026-09-21, a "Test" task that had just created an automation closed
 * `success: true` by leaving `build_target_path` out of the call.
 *
 * Evidence lives on the task in the readonly `verification_ref` /
 * `verification_hash` / `proof_level` family, written by the server operation
 * that obtained the proof, never by the model. This code READS it. It may carry
 * a SIBLING task's record for the same artifact over to the task being closed —
 * a record is about the artifact, not about whichever task named it when the
 * test ran — and it refuses a record the run has overwritten since (a re-draft
 * after the test). It never mints one.
 *
 * Refusal is a RESULT (`{success:false, reason:'unverified', missing:[…]}`), not
 * a throw: the model has to be able to act on it, and a throw reads to the loop
 * as an infrastructure fault.
 *
 * The predicates come from `agent-shared/finalize.js`, the copy both terminal
 * paths use. They were once duplicated here because importing the AGENT
 * HANDLER would have pulled the agent runtime into every task tick (and a
 * function's entry file is not importable at all); a shared sibling module is
 * neither, so there is one copy.
 * ────────────────────────────────────────────────────────────────────────── */

/**
 * The finalize policy of the agent behind this conversation, read the same way
 * the round budget is (agent_ref on the chat node, workspace from the envelope,
 * default 'functions'). An unreadable agent yields 'none': a gate we cannot read
 * is not a reason to refuse the write, and the old behaviour stands.
 */
async function readFinalizePolicy(workspace, chatPath) {
  return finalizePolicyOf(await readChatAgentProps(workspace, chatPath));
}

/**
 * Every raisin:AITask in the conversation. The gate reads siblings because the
 * record for an artifact may sit on the task that named it when it was tested.
 */
async function conversationTasks(workspace, chatPath) {
  const rows = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AITask'`,
    [chatPath],
  );
  return (Array.isArray(rows) ? rows : rows?.rows || []).map((r) => ({ id: r.id, path: r.path, props: r.properties || {} }));
}

/**
 * Decide a completion claim for a build task. Returns what the task will carry
 * if it may close (`adopt`), or why it may not (`missing`, `stale`).
 *
 * Every artifact the task is responsible for — its declared target and every
 * artifact it wrote in this run — needs a fresh record somewhere in the
 * conversation. The task's own record is preferred; a sibling's is adopted.
 * A SUPPORTING artifact (an agent, `target.kind`) has no verifier of its own
 * and is covered by a ready verification of an automation that calls it — see
 * `supportingEvidence` in run-evidence.js. The caller orders `targets` so the
 * primary is a directly verifiable one whenever the task has one.
 */
function decideCompletion({ taskPath, props, targets, tasks, run }) {
  const missing = [];
  const stale = [];
  let adopt = null;
  const primary = targets[0];

  for (const target of targets) {
    const isPrimary = target === primary;
    const ownGaps = isPrimary ? verificationGaps(props, target.spelling) : ['not the primary target'];
    const ownStale = isPrimary && ownGaps.length === 0 ? staleningWrite(props, target.artifact, run) : null;
    if (isPrimary && ownGaps.length === 0 && !ownStale) continue;

    const found = findEvidence(target.artifact, tasks, run, taskPath);
    stale.push(...found.stale);
    if (found.source) {
      if (isPrimary) adopt = { from: found.source.path, fields: evidenceFields(found.source.props) };
      continue;
    }
    if (target.kind) {
      const support = supportingEvidence(target.artifact, tasks, run, target.kind);
      if (support.covered) {
        if (isPrimary) adopt = { from: support.covered.source.path, fields: supportingEvidenceFields(target.spelling, support.covered) };
        continue;
      }
      stale.push(...support.reasons);
      missing.push(`a ready verification of an automation that calls ${target.spelling} — ${supportingHint(target.artifact)}`);
      continue;
    }
    if (isPrimary && ownStale) {
      missing.push(`a fresh verification of ${target.spelling} (the record on this task predates a later write)`);
    } else if (isPrimary && ownGaps.length > 0 && found.stale.length === 0) {
      missing.push(...ownGaps);
    } else {
      missing.push(`a verification record for ${target.spelling}`);
    }
  }
  return { missing: [...new Set(missing)], stale: [...new Set(stale)], adopt };
}

async function handler(input) {
  const { task_id, status, notes, build_target_path, __raisin_context } = input;
  const workspace = __raisin_context?.workspace || 'ai';
  const chatPath = __raisin_context?.chat_path;

  if (!task_id) throw new Error('task_id is required');
  if (!status) throw new Error('status is required');
  if (!VALID_STATUSES.includes(status)) {
    throw new Error(`Invalid status "${status}". Must be one of: ${VALID_STATUSES.join(', ')}`);
  }
  if (!chatPath) throw new Error('Missing chat_path in execution context');

  // Find the task by ID within this conversation
  const taskRows = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AITask' AND id = $2
     LIMIT 1`,
    [chatPath, task_id],
  );

  if (taskRows.length === 0) throw new Error(`Task not found: ${task_id}`);

  const taskNode = taskRows[0];
  const oldProps = taskNode.properties || {};
  const oldStatus = oldProps.status || 'pending';

  // Monotonic status guard: tool calls from one model turn execute in
  // parallel, so a batched in_progress update can arrive AFTER the completed
  // update for the same task. Never downgrade a terminal status.
  const TERMINAL_STATUSES = ['completed', 'cancelled'];
  if (TERMINAL_STATUSES.includes(oldStatus) && !TERMINAL_STATUSES.includes(status)) {
    const planPathIgnored = taskNode.path.split('/').slice(0, -1).join('/');
    const progressIgnored = await updatePlanProgress(workspace, planPathIgnored, chatPath, await readFinalizePolicy(workspace, chatPath));
    return {
      success: true,
      ignored: true,
      task_id,
      title: oldProps.title,
      old_status: oldStatus,
      new_status: oldStatus,
      plan_path: planPathIgnored,
      plan_id: progressIgnored?.plan_id || null,
      plan_status: progressIgnored?.status || null,
      total_tasks: progressIgnored?.total_tasks ?? null,
      completed_tasks: progressIgnored?.completed_tasks ?? null,
      pending_tasks: progressIgnored?.pending_tasks ?? null,
      message: `Task "${oldProps.title}" is already ${oldStatus}; downgrade to ${status} ignored.`,
    };
  }

  /* THE TASK MAY NAME ITS ARTIFACT HERE, not only at creation.
   *
   * An agent usually learns the path by CREATING the thing, which happens long
   * after create_plan wrote the task. Without this the field had no producer at
   * all: create-plan and add-task now accept it, but a plan written before the
   * artifact existed could never carry it, so `taskBuildTarget()` returned null
   * and the gate below was unreachable — it completed every build task freely.
   *
   * Declaring it is not an escape: the declaration is merged BEFORE the gate
   * reads it, so naming an artifact in the same call that claims completion is
   * gated exactly like naming it earlier. And not declaring is not an escape
   * either — see the derived targets below. */
  const declaredTarget = typeof build_target_path === 'string' && build_target_path
    ? build_target_path
    : null;
  let effectiveProps = declaredTarget && !taskBuildTarget(oldProps)
    ? { ...oldProps, build_target_path: declaredTarget }
    : oldProps;

  /* THE GATE. Only on the transition that makes a CLAIM, only for an agent that
   * declares the policy, only for a build task. */
  const finalizePolicy = await readFinalizePolicy(workspace, chatPath);
  const gateOn = finalizePolicy === FINALIZE_POLICY_VERIFIED;
  let run = null;
  let adoptedFrom = null;
  if (gateOn && status === 'completed') {
    run = await collectRunWrites(workspace, chatPath);

    /* A CHECK THAT COULD NOT RUN IS NOT A PASS. Without the run's tool calls
     * there is no telling whether this task wrote something, or whether its
     * record has been overwritten since — so no completion is accepted. */
    if (run.readable === false) {
      return {
        success: false,
        reason: 'unverified',
        task_id,
        title: oldProps.title,
        old_status: oldStatus,
        new_status: oldStatus,
        missing: ['a readable record of this run\'s tool calls'],
        message:
          `Task "${oldProps.title}" cannot be marked completed: this run's tool calls could not be read ` +
          `(${run.error || 'query failed'}), so the server cannot tell what it wrote or whether its evidence still holds.`,
      };
    }

    /* A WRITE STILL RUNNING IS NOT "NOTHING WRITTEN". Tool calls from one turn
     * run in parallel, so a completion batched beside a create can be decided
     * before the create's result row exists — and the write would then be
     * charged to no one. Refuse until the turn's other calls have answered. */
    const inflight = inFlightSiblings(run, __raisin_context?.msg_path);
    if (inflight.length > 0) {
      const names = [...new Set(inflight.map((c) => c.tool))].join(', ');
      return {
        success: false,
        reason: 'unverified',
        task_id,
        title: oldProps.title,
        old_status: oldStatus,
        new_status: oldStatus,
        in_flight: inflight.map((c) => c.tool),
        missing: [`the results of ${names}, called in the same turn and still running`],
        message:
          `Task "${oldProps.title}" cannot be marked completed in the same turn as ${names}: ` +
          'those calls have not returned yet, so the server cannot tell what they wrote. Call update-task again after their results arrive.',
      };
    }

    /* A PLAN-TIME GUESS IS SUPERSEDED BY THE REAL PATH. create-plan runs before
     * the artifact exists, so the path it names is a guess — measured: Builder
     * declared /automations/correct-homepage-title, then built and verified
     * /correct-homepage-titles, and the task could never close because the
     * guess (which it never wrote) stayed a target beside the real one. A new
     * declaration replaces the old one ONLY when nothing in this run wrote the
     * old target; anything the run did write is gated anyway as a derived
     * target below, so this cannot hide a write from the gate. */
    const oldDeclared = taskBuildTarget(oldProps);
    if (declaredTarget && oldDeclared && declaredTarget !== oldDeclared) {
      const oldArtifact = artifactOf(null, oldDeclared);
      const oldWasWritten = (run.writes || []).some((w) => sameArtifact(oldArtifact, artifactOf(w.workspace, w.path)));
      if (!oldWasWritten) effectiveProps = { ...effectiveProps, build_target_path: declaredTarget };
    }

    /* DERIVED TARGETS: what this task WROTE, whether or not it said so. */
    const derived = writesForTask(run, task_id);
    const declared = taskBuildTarget(effectiveProps);
    const targets = [];
    if (declared) {
      const artifact = artifactOf(null, declared);
      targets.push({ spelling: declared, artifact, derived: false, kind: supportingKind(artifact) });
    }
    for (const w of derived) {
      const known = targets.find((t) => sameArtifact(t.artifact, w));
      if (known) {
        /* A declaration names no workspace; the write does. Pin the target to
         * it, so `/agents/x` declared for an AUTOMATION stored at
         * automations:/agents/x is gated as that automation and not waved
         * through on the coverage of the functions-workspace agent of the same
         * name (review 2026-09-21). A write in another workspace then no longer
         * matches, and becomes a target of its own. */
        if (!known.artifact.workspace && w.workspace) {
          known.artifact = artifactOf(w.workspace, known.artifact.path);
          known.kind = supportingKind(known.artifact, w);
        } else {
          known.kind = known.kind || supportingKind(known.artifact, w);
        }
        continue;
      }
      const artifact = artifactOf(w.workspace, w.path);
      targets.push({ spelling: w.path, artifact, derived: true, write: w, kind: supportingKind(artifact, w) });
    }
    /* A supporting artifact is not what a record is ROUTED by — verify-automation
     * stamps the task that names the automation — so an agent is the primary
     * target only when the task built nothing directly verifiable. */
    targets.sort((a, b) => (a.kind ? 1 : 0) - (b.kind ? 1 : 0));

    if (targets.length > 0) {
      const primary = targets[0];
      // The declaration persists even when the claim is refused: it is what
      // routes the next test's evidence to this task.
      const declaredKind = declared ? supportingKind(artifactOf(null, declared)) : null;
      if (!taskBuildTarget(effectiveProps) || (declaredKind && !primary.kind)) {
        effectiveProps = { ...effectiveProps, build_target_path: primary.spelling };
      }
      const tasks = await conversationTasks(workspace, chatPath);
      const decision = decideCompletion({ taskPath: taskNode.path, props: effectiveProps, targets, tasks, run });

      if (decision.missing.length > 0) {
        const declarationChanged = effectiveProps.build_target_path !== oldProps.build_target_path;
        if (declarationChanged) {
          await raisin.nodes.update(workspace, taskNode.path, {
            properties: { ...oldProps, build_target_path: effectiveProps.build_target_path },
          });
        }
        const wrote = targets.filter((t) => t.derived).map((t) => ({
          artifact: describeArtifact(t.artifact),
          tool: t.write.tool,
          at: t.write.at,
        }));
        return {
          success: false,
          reason: 'unverified',
          task_id,
          title: oldProps.title,
          old_status: oldStatus,
          new_status: oldStatus,
          build_target: primary.spelling,
          build_targets: targets.map((t) => t.spelling),
          ...(wrote.length ? { derived_from_writes: wrote } : {}),
          ...(decision.stale.length ? { stale: decision.stale } : {}),
          build_target_persisted: effectiveProps.build_target_path,
          missing: decision.missing,
          message:
            `Task "${oldProps.title}" builds ${targets.map((t) => t.spelling).join(', ')}` +
            (wrote.length ? ` (written in this run by ${wrote.map((w) => w.tool).join(', ')}, so it is a build task whether or not it declared one)` : '') +
            ` and has no current verification record, so it cannot be marked completed. Missing: ${decision.missing.join('; ')}.` +
            (decision.stale.length ? ` ${decision.stale.join('; ')}.` : '') +
            ' Obtain the proof (validate, then test execution) and the verification record will be written by the operation that obtains it — not by this tool.' +
            /* The order matters and is not obvious: a verification stamps only
             * the tasks that ALREADY name its artifact. Measured: a target set
             * by this very refusal left a verification made a moment earlier
             * stamped on no task at all. */
            (declarationChanged
              ? ` This task now names ${effectiveProps.build_target_path}; a verification stamps only tasks that already name its artifact, so run the verification for ${effectiveProps.build_target_path} again NOW, then mark this task completed.`
              : '') +
            ' If the work was abandoned, mark the task failed or cancelled instead.',
          ...(declarationChanged ? { next_step: `verify ${effectiveProps.build_target_path} again, then update-task completed` } : {}),
        };
      }

      if (decision.adopt) {
        /* Carry the record over, with the envelope spelled as THIS task spells
         * its target — the gate and the terminal statement compare the two
         * strings exactly, and the artifact was already matched above. */
        const fields = { ...decision.adopt.fields };
        const ref = fields.verification_ref;
        if (ref && typeof ref === 'object') fields.verification_ref = { ...ref, 'raisin:path': primary.spelling };
        else fields.verification_ref = primary.spelling;
        effectiveProps = { ...effectiveProps, ...fields, verification_source_task: decision.adopt.from };
        adoptedFrom = decision.adopt.from;
      }
    }
  }

  // Build updated properties
  const updatedProps = { ...effectiveProps, status };
  if (notes) updatedProps.completion_notes = notes;

  await raisin.nodes.update(workspace, taskNode.path, { properties: updatedProps });

  // Propagate progress to parent plan
  const planPath = taskNode.path.split('/').slice(0, -1).join('/');
  const progress = await updatePlanProgress(workspace, planPath, chatPath, finalizePolicy, run);

  return {
    success: true,
    task_id,
    title: oldProps.title,
    old_status: oldStatus,
    new_status: status,
    ...(adoptedFrom ? { evidence_from_task: adoptedFrom } : {}),
    plan_path: planPath,
    plan_id: progress?.plan_id || null,
    plan_status: progress?.status || null,
    total_tasks: progress?.total_tasks ?? null,
    completed_tasks: progress?.completed_tasks ?? null,
    pending_tasks: progress?.pending_tasks ?? null,
    message: `Task "${oldProps.title}" marked as ${status}`,
  };
}

/* Exported for `tests/finalize-gate.test.js`. The handler is the contract; the
 * predicates are exported because the gate's whole job is deciding, and a
 * decision is worth testing without a server. */
export { handler, updatePlanProgress, taskBuildTarget, verificationGaps, readFinalizePolicy, decideCompletion, FINALIZE_POLICY_VERIFIED, PROOF_LEVELS };
