/**
 * Notification store for real-time alerts.
 *
 * Single source of truth for notification badge count.
 * Handles cascading mark-as-read for message notifications.
 */
import { writable, derived, get } from 'svelte/store';
import { browser } from '$app/environment';
import { query, getDatabase } from '$lib/raisin';
import { user } from './auth';
import { messagingStore } from './messaging-store';
import { toastStore } from './toast';

const ACCESS_CONTROL = 'raisin:access_control';

export interface Notification {
    id: string;
    path: string;
    properties: {
        type: 'message' | 'relationship_request' | 'system' | 'chat';
        title: string;
        body?: string;
        link?: string;
        read: boolean;
        data?: any;
    };
}

function createNotificationStore() {
    const { subscribe, set, update } = writable<Notification[]>([]);
    let unsubscribe: (() => void) | null = null;

    /**
     * Extract conversation ID from a notification link path.
     * Link format: /users/{userId}/inbox/chats/{conversationId}
     */
    function extractConversationId(link: string | undefined): string | null {
        if (!link) return null;
        const parts = link.split('/');
        const chatsIndex = parts.indexOf('chats');
        if (chatsIndex !== -1 && parts.length > chatsIndex + 1) {
            return parts[chatsIndex + 1];
        }
        return null;
    }

    return {
        subscribe,

        async init() {
            const currentUser = get(user);
            if (!currentUser?.home || unsubscribe) return;

            const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
            const notifsPath = `${homePath}/inbox/notifications`;

            // 1. Initial Load - get ALL notifications (both read and unread for dropdown)
            try {
                const results = await query<Notification>(`
                    SELECT id, path, properties FROM '${ACCESS_CONTROL}'
                    WHERE CHILD_OF('${notifsPath}')
                    ORDER BY path DESC
                    LIMIT 50
                `);
                set(results);
            } catch (e) {
                console.error('[notifications] Load failed', e);
            }

            // 2. Real-time Subscription with includeNode for instant updates
            const db = await getDatabase();
            const ws = db.workspace(ACCESS_CONTROL);
            const subscription = await ws.events().subscribeToPath(
                `${notifsPath}/*`,
                (event) => {
                    const payload = event.payload as any;
                    const kind = payload?.kind;
                    const node = payload?.node;

                    if (kind === 'Created' && node && node.node_type === 'raisin:Notification') {
                        update(n => [node as unknown as Notification, ...n]);

                        // Show toast notification
                        if (browser) {
                            const notif = node as unknown as Notification;
                            toastStore.show({
                                type: notif.properties.type,
                                title: notif.properties.title,
                                body: notif.properties.body,
                                link: notif.properties.link,
                            });
                        }
                    } else if (kind === 'Updated' && node) {
                        // Update notification in place (e.g., when marked as read)
                        update(n => n.map(item =>
                            item.id === node.id
                                ? { ...item, properties: node.properties }
                                : item
                        ));
                    } else if (kind === 'Deleted') {
                        // Remove deleted notification from local state
                        const nodeId = payload?.node_id;
                        if (nodeId) {
                            update(n => n.filter(item => item.id !== nodeId));
                        }
                    }
                },
                { includeNode: true }
            );

            unsubscribe = () => subscription.unsubscribe();
        },

        /**
         * Mark a notification as read.
         * For message notifications, also marks the conversation as read.
         */
        async markAsRead(id: string) {
            const currentUser = get(user);
            if (!currentUser) return;

            // Find the notification to get its type and link
            const notifications = get({ subscribe });
            const notification = notifications.find(n => n.id === id);

            if (!notification || notification.properties.read) return;

            try {
                // 1. Mark notification as read in database
                await query(`
                    UPDATE '${ACCESS_CONTROL}'
                    SET properties = properties || '{"read": true}'
                    WHERE id = $1
                `, [id]);

                // 2. Update local state
                update(n => n.map(item =>
                    item.id === id
                        ? { ...item, properties: { ...item.properties, read: true } }
                        : item
                ));

                // 3. Cascade: For message notifications, also mark conversation as read
                if (notification.properties.type === 'message') {
                    const convId = extractConversationId(notification.properties.link);
                    if (convId) {
                        await messagingStore.markAsRead(convId);
                    }
                }
            } catch (e) {
                console.error('[notifications] Mark read failed', e);
            }
        },

        /**
         * Mark all notifications as read.
         */
        async markAllAsRead() {
            const currentUser = get(user);
            if (!currentUser?.home) return;

            const notifications = get({ subscribe });
            const unreadNotifications = notifications.filter(n => !n.properties.read);

            if (unreadNotifications.length === 0) return;

            try {
                const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
                const notifsPath = `${homePath}/inbox/notifications`;

                // Batch update all unread notifications
                await query(`
                    UPDATE '${ACCESS_CONTROL}'
                    SET properties = properties || '{"read": true}'
                    WHERE CHILD_OF('${notifsPath}')
                      AND properties->>'read' = 'false'
                `);

                // Update local state
                update(n => n.map(item => ({
                    ...item,
                    properties: { ...item.properties, read: true }
                })));

                // Cascade: Mark all related conversations as read
                for (const notif of unreadNotifications) {
                    if (notif.properties.type === 'message') {
                        const convId = extractConversationId(notif.properties.link);
                        if (convId) {
                            await messagingStore.markAsRead(convId);
                        }
                    }
                }
            } catch (e) {
                console.error('[notifications] Mark all read failed', e);
            }
        },

        /**
         * Delete a single notification.
         */
        async delete(id: string) {
            const currentUser = get(user);
            if (!currentUser) return;

            try {
                // Delete from database
                await query(`
                    DELETE FROM '${ACCESS_CONTROL}'
                    WHERE id = $1
                `, [id]);

                // Update local state (will also be updated by WebSocket event)
                update(n => n.filter(item => item.id !== id));
            } catch (e) {
                console.error('[notifications] Delete failed', e);
            }
        },

        /**
         * Clear all notifications (delete all).
         */
        async clearAll() {
            const currentUser = get(user);
            if (!currentUser?.home) return;

            const notifications = get({ subscribe });
            if (notifications.length === 0) return;

            try {
                const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
                const notifsPath = `${homePath}/inbox/notifications`;

                // Delete all notifications from database
                await query(`
                    DELETE FROM '${ACCESS_CONTROL}'
                    WHERE CHILD_OF('${notifsPath}')
                `);

                // Clear local state
                set([]);
            } catch (e) {
                console.error('[notifications] Clear all failed', e);
            }
        },

        reset() {
            if (unsubscribe) {
                unsubscribe();
                unsubscribe = null;
            }
            set([]);
        }
    };
}

export const notificationStore = createNotificationStore();

/** Derived store: count of unread notifications */
export const unreadCount = derived(notificationStore, $notifications =>
    $notifications.filter(n => !n.properties.read).length
);

/** Derived store: just the unread notifications */
export const unreadNotifications = derived(notificationStore, $notifications =>
    $notifications.filter(n => !n.properties.read)
);
