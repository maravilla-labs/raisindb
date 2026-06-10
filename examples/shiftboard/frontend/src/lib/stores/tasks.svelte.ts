/**
 * Inbox tasks (Svelte 5 runes) — the human-in-the-loop panel.
 *
 * Tasks are plain `raisin:InboxTask` nodes that workflows create under the
 * logged-in user's home inbox (`${home}/inbox/...` in the
 * `raisin:access_control` workspace). No special UI framework is needed:
 *
 *   SSR seed   GET /api/inbox/{repo}?status=pending (+page.server.ts)
 *   Live       the shared inbox node subscription (notifications store)
 *              forwards node:created / node:updated events here
 *   Complete   POST /api/inbox/{repo}/tasks/{id}/complete via InboxApi —
 *              optimistic removal with rollback on error
 *
 * SDK APIs used:
 *   InboxApi.listTasks({ status: 'pending' })
 *   InboxApi.completeTask(taskId, { action })
 */
import type { EventMessage, InboxTask } from '@raisindb/client';
import { getClient, getInbox } from '../raisin';

/** Minimal node shape carried by inbox events (include_node: true). */
interface InboxEventNode {
  id?: string;
  path?: string;
  node_type?: string;
  properties?: Record<string, unknown>;
}

class TaskState {
  /** Pending tasks for the logged-in user, server-sorted (priority, due). */
  tasks = $state<InboxTask[]>([]);
  error = $state<string | null>(null);
  /** Bumped when the bell asks the panel to scroll into view. */
  focusSeq = $state(0);

  #connected = false;

  /** Seed from SSR data. Runs during server render and again on hydration. */
  seed(tasks: InboxTask[]): void {
    this.tasks = tasks;
    this.error = null;
  }

  /** Register reconnect resync + polling fallback. Call once after hydration. */
  connect(): void {
    if (this.#connected) return;
    this.#connected = true;
    // Live events keep the list fresh; after an offline gap, missed events
    // are gone — refetch the pending list.
    getClient().onReconnected(() => {
      this.refresh().catch(() => {});
    });
    // Fallback poll: the flow engine currently writes task nodes through the
    // storage layer directly (create_deep_node), which does NOT publish
    // node:created/node:updated events — only NodeService writes do (e.g.
    // chat messages). Until the server emits events for flow-created nodes,
    // a slow poll keeps the panel honest; the subscription path above it is
    // the intended mechanism and takes over as soon as events arrive.
    setInterval(() => {
      this.refresh().catch(() => {});
    }, 30_000);
  }

  /** Re-fetch pending tasks (reconnect resync). */
  async refresh(): Promise<void> {
    const { tasks } = await getInbox().listTasks({ status: 'pending' });
    this.tasks = tasks;
  }

  /** Header bell clicked: ask the TaskPanel to scroll into view + flash. */
  requestFocus(): void {
    if (this.tasks.length > 0) this.focusSeq += 1;
  }

  /**
   * Fed by the shared `${home}/inbox/**` subscription (notifications store).
   * A created/updated node with status `pending` is upserted; any other
   * status (completed/expired/cancelled) removes the card.
   */
  onInboxEvent(event: EventMessage): void {
    const node = (event.payload as { node?: InboxEventNode } | undefined)?.node;
    if (!node?.path || node.node_type !== 'raisin:InboxTask') return;

    const task = { ...(node.properties ?? {}), id: node.id, path: node.path } as InboxTask;
    const idx = this.tasks.findIndex((t) => t.path === task.path);

    if (task.status === 'pending') {
      if (idx >= 0) this.tasks[idx] = task;
      else this.tasks = [task, ...this.tasks];
    } else if (idx >= 0) {
      this.tasks = this.tasks.filter((t) => t.path !== task.path);
    }
  }

  /**
   * Complete a task with the chosen option value (one button per option).
   * Optimistic: the card disappears immediately; on failure it is restored
   * and the error shown. The response lands in the workflow as
   * `__human_response.action`.
   */
  async complete(taskId: string, value: string): Promise<void> {
    const idx = this.tasks.findIndex((t) => t.id === taskId);
    if (idx < 0) return;
    const removed = this.tasks[idx];

    this.error = null;
    this.tasks = this.tasks.filter((t) => t.id !== taskId);

    try {
      await getInbox().completeTask(taskId, { action: value });
    } catch (err) {
      // Roll back the optimistic removal at the original position.
      const restored = [...this.tasks];
      restored.splice(Math.min(idx, restored.length), 0, removed);
      this.tasks = restored;
      this.error = err instanceof Error ? err.message : String(err);
    }
  }
}

export const tasks = new TaskState();
