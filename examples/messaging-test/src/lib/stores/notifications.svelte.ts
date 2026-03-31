/**
 * Notifications store using Svelte 5 runes for real-time notification subscriptions.
 */
import { browser } from '$app/environment';
import { query, subscribeToPath, ACCESS_CONTROL_WORKSPACE, getUser } from '$lib/raisin';
import { toastStore } from './toast.svelte';

export interface Notification {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    type: string;
    title: string;
    body?: string;
    link?: string;
    read: boolean;
    priority?: number;
    data?: Record<string, unknown>;
    created_at?: string;
    expires_at?: string;
  };
}

// Module-level state using $state rune (Svelte 5)
let notifications = $state<Notification[]>([]);
let lastCheckedAt = $state<string>(new Date().toISOString());
let loading = $state<boolean>(false);
let initialized = $state<boolean>(false);
let subscriptionCleanup: (() => void) | null = null;

// Derived state
function getUnreadCount(): number {
  return notifications.filter((n) => !n.properties?.read).length;
}

function getUserHomePath(): string | null {
  const currentUser = getUser();
  if (!currentUser?.home) return null;
  // user.home is like /raisin:access_control/users/abc123
  return currentUser.home.replace(`/${ACCESS_CONTROL_WORKSPACE}`, '');
}

async function loadNotifications(): Promise<void> {
  const homePath = getUserHomePath();
  if (!homePath) return;

  try {
    const notificationsPath = `${homePath}/inbox/notifications`;
    const result = await query<Notification>(`
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL_WORKSPACE}'
      WHERE DESCENDANT_OF('${notificationsPath}')
        AND node_type = 'raisin:Notification'
      ORDER BY properties->>'created_at' DESC
      LIMIT 50
    `);
    notifications = result;
  } catch (err) {
    console.error('[notifications] Failed to load notifications:', err);
  }
}

async function setupSubscription(): Promise<void> {
  if (!browser) return;

  const homePath = getUserHomePath();
  if (!homePath) return;

  try {
    const notificationsPath = `${homePath}/inbox/notifications`;
    subscriptionCleanup = await subscribeToPath(notificationsPath, (event) => {
      // When a new notification arrives, reload and show toast
      loadNotifications().then(() => {
        if (event.kind === 'created' && event.node) {
          const node = event.node as unknown as Notification;
          const props = node.properties;
          if (props && !props.read) {
            // Show toast for new notification
            toastStore.add(props.title, props.body || '', props.link);
          }
        }
      });
    });
  } catch (err) {
    console.error('[notifications] Failed to setup subscription:', err);
  }
}

// Exported store object
export const notificationStore = {
  // Getters for reactive state
  get notifications() {
    return notifications;
  },
  get unreadCount() {
    return getUnreadCount();
  },
  get loading() {
    return loading;
  },
  get initialized() {
    return initialized;
  },

  async init() {
    if (!browser) return;

    loading = true;
    try {
      await loadNotifications();
      await setupSubscription();
      initialized = true;
    } catch (err) {
      console.error('[notifications] Init error:', err);
    } finally {
      loading = false;
    }
  },

  async refresh() {
    await loadNotifications();
  },

  async markAsRead(notificationPath: string) {
    // Update local state optimistically
    const idx = notifications.findIndex((n) => n.path === notificationPath);
    if (idx !== -1) {
      notifications[idx] = {
        ...notifications[idx],
        properties: {
          ...notifications[idx].properties,
          read: true,
        },
      };
    }

    // TODO: Call raisin API to update the notification node
    // This would typically be: await raisin.nodes.update_property(workspace, path, 'read', true);
  },

  async markAllAsRead() {
    // Update local state optimistically
    notifications = notifications.map((n) => ({
      ...n,
      properties: {
        ...n.properties,
        read: true,
      },
    }));

    // TODO: Call raisin API to update all notification nodes
  },

  reset() {
    notifications = [];
    lastCheckedAt = new Date().toISOString();
    loading = false;
    initialized = false;
    subscriptionCleanup?.();
    subscriptionCleanup = null;
  },

  cleanup() {
    subscriptionCleanup?.();
    subscriptionCleanup = null;
  },
};
