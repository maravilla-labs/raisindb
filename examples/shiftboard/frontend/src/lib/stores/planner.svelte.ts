/**
 * Planner tab state: the coordinator agent chat + plan approval helpers.
 *
 * A SECOND AgentChatState instance (separate ConversationStore, separate
 * conversation) scoped to /agents/shift-coordinator — the plan-enabled agent
 * (execution_mode approve_then_auto). The Board tab's `chat` store keeps its
 * own conversation with /agents/shift-planner untouched.
 *
 * Two event-ordering gotchas of the plan system are handled here with
 * RELOAD POLLING (mirroring the admin console's scheduled plan refreshes and
 * plan-modes-test's pollSdkPlan):
 *
 * 1. Proposal grace: the backend's safety-net `done` event (finishReason
 *    awaiting_plan_approval) can arrive BEFORE the asynchronously delivered
 *    ai_plan card message. The SDK's deferred reload stops retrying as soon
 *    as ANY new message appears — possibly still before the card landed. So
 *    while the turn rests on awaiting_plan_approval without a pending plan
 *    in the projection, keep reloading until the card shows up.
 *
 * 2. Execution progress: after approvePlan() the agent continues WITHOUT a
 *    user message. The plan/task lifecycle messages (ai_plan/ai_task_update)
 *    are delivered into the conversation asynchronously and are not reliably
 *    accompanied by live events on the user's subscription — so after an
 *    approve/reject we poll loadMessages() until the plan reaches a terminal
 *    status (completed/failed/cancelled).
 */
import type { ConversationStoreSnapshot } from '@raisindb/client';
import { COORDINATOR_PATH } from '../agent';
import { AgentChatState } from './chat.svelte';

const POLL_INTERVAL_MS = 2000;
const GRACE_MAX_TRIES = 10; // proposal card grace window (~20s)
const WATCH_MAX_TRIES = 180; // execution watch window (~6min)

const TERMINAL = new Set(['completed', 'failed', 'cancelled']);

function hasPendingPlan(s: ConversationStoreSnapshot): boolean {
  return s.plans.some((p) => p.status === 'pending_approval');
}

function lastFinishReason(s: ConversationStoreSnapshot): string | undefined {
  // Skip trailing delivery-mirror messages that carry no finishReason.
  for (let i = s.messages.length - 1; i >= 0; i--) {
    const m = s.messages[i];
    if (m.role === 'assistant' && m.finishReason) return m.finishReason;
  }
  return undefined;
}

class PlannerChatState extends AgentChatState {
  #timer: ReturnType<typeof setTimeout> | null = null;
  #triesLeft = 0;
  /** Predicate: keep polling while it returns true. */
  #keepPolling: ((s: ConversationStoreSnapshot) => boolean) | null = null;

  constructor() {
    super(COORDINATOR_PATH, (s) => this.#onSnapshot(s));
  }

  async approvePlan(planPath: string): Promise<void> {
    await super.approvePlan(planPath);
    // Execution continues without further user messages — watch the plan
    // to a terminal state via reloads.
    this.#startPolling(WATCH_MAX_TRIES, (s) => {
      const plan = s.plans.find((p) => p.planPath === planPath);
      return !plan || !TERMINAL.has(plan.status);
    });
  }

  async rejectPlan(planPath: string, feedback?: string): Promise<void> {
    await super.rejectPlan(planPath, feedback);
    this.#startPolling(WATCH_MAX_TRIES, (s) => {
      const plan = s.plans.find((p) => p.planPath === planPath);
      return !plan || !TERMINAL.has(plan.status);
    });
  }

  /** Proposal grace: safety-net done seen, but the ai_plan card not yet. */
  #onSnapshot(s: ConversationStoreSnapshot): void {
    if (this.#timer || this.#keepPolling) return; // already polling
    const needsGrace =
      !s.isStreaming && lastFinishReason(s) === 'awaiting_plan_approval' && !hasPendingPlan(s);
    if (needsGrace) {
      this.#startPolling(GRACE_MAX_TRIES, (snap) => !hasPendingPlan(snap));
    }
  }

  #startPolling(
    maxTries: number,
    keepPolling: (s: ConversationStoreSnapshot) => boolean,
  ): void {
    this.#triesLeft = Math.max(this.#triesLeft, maxTries);
    this.#keepPolling = keepPolling;
    if (!this.#timer) this.#tick();
  }

  #tick(): void {
    this.#timer = setTimeout(async () => {
      this.#timer = null;
      const done = !this.#keepPolling || !this.#keepPolling(this.snapshot);
      if (done || this.#triesLeft-- <= 0) {
        this.#keepPolling = null;
        this.#triesLeft = 0;
        return;
      }
      // Skip the fetch while a live stream is active (events are flowing),
      // but keep the loop alive to re-check after it settles.
      if (!this.snapshot.isStreaming) {
        await this.reload().catch(() => {});
      }
      this.#tick();
    }, POLL_INTERVAL_MS);
  }
}

export const planner = new PlannerChatState();
