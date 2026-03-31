/**
 * Presence store for real-time user status.
 *
 * Tracks online status and active conversation to prevent
 * unnecessary notifications when user is already viewing a chat.
 */
import { writable, get } from 'svelte/store';
import { getDatabase, query } from '$lib/raisin';
import { user } from './auth';

const WORKSPACE = 'launchpad';

export interface PresenceInfo {
    userId: string; // Global user id
    status: 'online' | 'away' | 'offline';
    lastSeen: string;
    activeConversation?: string; // Currently viewing this conversation ID
}

// Track the active conversation locally
let currentActiveConversation: string | null = null;

function createPresenceStore() {
    const { subscribe, set, update } = writable<Map<string, PresenceInfo>>(new Map());
    let unsubscribe: (() => void) | null = null;
    let heartbeatInterval: any = null;

    return {
        subscribe,

        async init() {
            const currentUser = get(user);
            if (!currentUser?.home || unsubscribe) return;

            const db = await getDatabase();
            const ws = db.workspace(WORKSPACE);
            const presencePath = `/presence`;
            
            const myPathName = currentUser.id || '';

            // 1. Initial Load
            try {
                const results = await query<any>(`
                    SELECT name as userId, properties->>'status' as status, properties->>'last_seen' as lastSeen
                    FROM ${WORKSPACE}
                    WHERE CHILD_OF('${presencePath}')
                      AND node_type = 'launchpad:Presence'
                `);

                const initialMap = new Map();
                results.forEach(r => initialMap.set(r.userId, r));
                set(initialMap);
            } catch (_e) {
                // Initial load empty is expected
            }

            // 2. Real-time Subscription
            const subscription = await ws.events().subscribeToPath(`${presencePath}/*`, async (event) => {
                // Fix: Safety check for event path
                if (!event?.path) return;

                const userId = event.path.split('/').pop();
                if (!userId || userId === myPathName) return;

                if (event.kind === 'Created' || event.kind === 'Updated') {
                    try {
                        const node = await ws.nodes().getByPath(event.path);
                        if (node && node.node_type === 'launchpad:Presence') {
                            update(map => {
                                map.set(userId, {
                                    userId,
                                    status: node.properties.status as any,
                                    lastSeen: node.properties.last_seen
                                });
                                return new Map(map);
                            });
                        }
                    } catch (err) {
                        console.error('[presence] Failed to fetch node:', event.path, err);
                    }
                } else if (event.kind === 'Deleted') {
                    update(map => {
                        map.delete(userId);
                        return new Map(map);
                    });
                }
            });

            unsubscribe = () => subscription.unsubscribe();

            // 3. Heartbeat
            this.heartbeat();
            heartbeatInterval = setInterval(() => this.heartbeat(), 10000); // Heartbeat every 10s
        },

        async heartbeat() {
            const currentUser = get(user);
            if (!currentUser?.home) return;

            const db = await getDatabase();
            const myPathName = currentUser.id;
            const myPresencePath = `/presence/${myPathName}`;

            // Build presence data including active conversation if any
            const presenceData: Record<string, any> = {
                status: 'online',
                last_seen: new Date().toISOString()
            };

            // Include active conversation if user is viewing one
            if (currentActiveConversation) {
                presenceData.active_conversation = currentActiveConversation;
            }

            const sql = `UPSERT INTO ${WORKSPACE} (path, node_type, properties) VALUES ($1, 'launchpad:Presence', $2::jsonb)`;
            const params = [myPresencePath, JSON.stringify(presenceData)];

            try {
                await db.executeSql(sql, params);
            } catch (e) {
                console.error('[presence] Heartbeat failed', e);
            }
        },

        /**
         * Set the active conversation (user is viewing this chat).
         * Call this when opening a chat popup or viewing inbox conversation.
         */
        setActiveConversation(conversationId: string | null) {
            currentActiveConversation = conversationId;
            // Immediately send updated presence
            this.heartbeat();
        },

        /**
         * Get the current active conversation.
         */
        getActiveConversation(): string | null {
            return currentActiveConversation;
        },

        isOnline(identifier: string): boolean {
            const name = identifier.includes('/') ? identifier.split('/').pop() || '' : identifier;
            const info = get({ subscribe }).get(name);
            if (!info) return false;
            
            const lastSeen = new Date(info.lastSeen).getTime();
            const now = Date.now();
            // Faster timeout for testing: 15 seconds
            return (now - lastSeen) < 15000;
        },

        reset() {
            if (unsubscribe) {
                unsubscribe();
                unsubscribe = null;
            }
            if (heartbeatInterval) {
                clearInterval(heartbeatInterval);
                heartbeatInterval = null;
            }
            set(new Map());
        }
    };
}

export const presenceStore = createPresenceStore();
