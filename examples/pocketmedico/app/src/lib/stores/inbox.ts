import { writable, derived, get } from 'svelte/store';
import { currentUser } from './auth';

export type InboxItemType =
	| 'order_status_update' // For customers
	| 'order_ready' // For customers
	| 'new_review_task' // For nurses
	| 'review_reminder'; // For nurses

export interface InboxItem {
	id: string;
	userId: string;
	type: InboxItemType;
	title: string;
	message: string;
	read: boolean;
	orderId?: string;
	createdAt: Date;
}

// Initial demo inbox items
const initialInboxItems: InboxItem[] = [
	{
		id: 'inbox-1',
		userId: 'customer-demo',
		type: 'order_ready',
		title: 'Transcription Ready',
		message: 'Your transcription for order PM-202501-1001 is ready for download.',
		read: false,
		orderId: 'order-1',
		createdAt: new Date(Date.now() - 1 * 24 * 60 * 60 * 1000)
	},
	{
		id: 'inbox-2',
		userId: 'customer-demo',
		type: 'order_status_update',
		title: 'Order Processing',
		message: 'Your order PM-202501-1002 is currently being processed.',
		read: true,
		orderId: 'order-2',
		createdAt: new Date(Date.now() - 4 * 60 * 60 * 1000)
	},
	{
		id: 'inbox-3',
		userId: 'customer-demo',
		type: 'order_status_update',
		title: 'Welcome to Pocket Medico',
		message: 'Thank you for joining! Start by creating your first transcription order.',
		read: true,
		createdAt: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000)
	},
	{
		id: 'inbox-4',
		userId: 'nurse-demo',
		type: 'new_review_task',
		title: 'New Review Task',
		message: 'A new transcription (PM-202501-1004) is ready for your review.',
		read: false,
		orderId: 'order-4',
		createdAt: new Date(Date.now() - 1 * 60 * 60 * 1000)
	},
	{
		id: 'inbox-5',
		userId: 'nurse-demo',
		type: 'new_review_task',
		title: 'Urgent Review Task',
		message: 'An urgent transcription (PM-202501-1005) needs your immediate attention.',
		read: false,
		orderId: 'order-5',
		createdAt: new Date(Date.now() - 30 * 60 * 1000)
	},
	{
		id: 'inbox-6',
		userId: 'nurse-demo',
		type: 'review_reminder',
		title: 'Review Reminder',
		message: 'You have 2 pending reviews in your queue.',
		read: true,
		createdAt: new Date(Date.now() - 2 * 60 * 60 * 1000)
	}
];

// Inbox items store
export const inboxItems = writable<InboxItem[]>(initialInboxItems);

// Derived: inbox for current user
export const userInbox = derived([inboxItems, currentUser], ([$items, $user]) =>
	$user
		? $items.filter((i) => i.userId === $user.id).sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
		: []
);

// Derived: unread count
export const unreadCount = derived(userInbox, ($inbox) => $inbox.filter((i) => !i.read).length);

// Mark item as read
export function markAsRead(itemId: string): void {
	inboxItems.update((items) =>
		items.map((item) => (item.id === itemId ? { ...item, read: true } : item))
	);
}

// Mark all as read
export function markAllAsRead(): void {
	const user = get(currentUser);
	if (!user) return;

	inboxItems.update((items) =>
		items.map((item) => (item.userId === user.id ? { ...item, read: true } : item))
	);
}

// Add inbox notification
export function addInboxItem(data: Omit<InboxItem, 'id' | 'createdAt' | 'read'>): void {
	const item: InboxItem = {
		...data,
		id: crypto.randomUUID(),
		read: false,
		createdAt: new Date()
	};
	inboxItems.update((items) => [item, ...items]);
}

// Delete inbox item
export function deleteInboxItem(itemId: string): void {
	inboxItems.update((items) => items.filter((item) => item.id !== itemId));
}
