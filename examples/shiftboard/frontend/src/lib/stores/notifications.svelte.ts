/**
 * Inbox notifications (Svelte 5 runes).
 *
 * Pure node subscription — no polling, no extra API. The messaging
 * pipeline delivers items into the logged-in user's home inbox in the
 * `raisin:access_control` workspace; we subscribe to node:created under
 * `${user.home}/inbox` and bump the bell badge / show a toast.
 *
 * SDK API used:
 *   db.workspace('raisin:access_control').events().subscribe(
 *     { path: `${home}/inbox/**`, event_types: ['node:created', 'node:updated'],
 *       include_node: true },
 *     callback)
 *
 * Path filter semantics: `*` matches exactly one path segment, `**` matches
 * any depth — we want `**` because inbox items nest (e.g. chats/<conv>/<msg>).
 *
 * This is the app's single inbox subscription: it also feeds the tasks store
 * (raisin:InboxTask nodes → TaskPanel) — `node:updated` is subscribed so a
 * task completed elsewhere disappears from the panel live.
 */
import type { EventMessage, IdentityUser } from '@raisindb/client';
import { getDb } from '../raisin';
import { tasks } from './tasks.svelte';

const ACCESS_CONTROL = 'raisin:access_control';

export interface Toast {
  id: number;
  title: string;
}

class NotificationState {
  unread = $state(0);
  toasts = $state<Toast[]>([]);

  #initialized = false;
  #nextToastId = 1;

  /** Start the inbox subscription. Call once after login. */
  async init(user: IdentityUser): Promise<void> {
    if (this.#initialized || !user.home) return;
    this.#initialized = true;

    // user.home may or may not carry the workspace prefix depending on the
    // auth path; the subscription filter expects a workspace-relative path.
    const home = user.home.replace(new RegExp(`^/${ACCESS_CONTROL}`), '');

    try {
      await getDb().workspace(ACCESS_CONTROL).events().subscribe(
        {
          path: `${home}/inbox/**`,
          event_types: ['node:created', 'node:updated'],
          include_node: true,
        },
        (event) => this.#onInboxEvent(event),
      );
    } catch (err) {
      console.warn('[notifications] inbox subscription failed:', err);
    }
  }

  #onInboxEvent(event: EventMessage): void {
    // Keep the task panel in sync (created → card appears, updated with a
    // non-pending status → card disappears).
    tasks.onInboxEvent(event);

    // The bell only counts NEW inbox items, not updates to existing ones.
    if (event.event_type !== 'node:created') return;

    const payload = event.payload as
      | { node?: { name?: string; properties?: Record<string, unknown> } }
      | undefined;
    const props = payload?.node?.properties ?? {};

    // The user's own outgoing chat messages are persisted under their inbox
    // too (role: 'user') — don't notify about things the user just did.
    if (props.role === 'user') return;

    this.unread += 1;
    this.#showToast(
      (props.title as string) ??
        (props.subject as string) ??
        payload?.node?.name ??
        'New inbox item',
    );
  }

  #showToast(title: string): void {
    const toast: Toast = { id: this.#nextToastId++, title };
    this.toasts.push(toast);
    // Auto-dismiss after a few seconds.
    setTimeout(() => {
      this.toasts = this.toasts.filter((t) => t.id !== toast.id);
    }, 5000);
  }

  /** Clicking the bell clears the badge. */
  clear(): void {
    this.unread = 0;
  }
}

export const notifications = new NotificationState();
