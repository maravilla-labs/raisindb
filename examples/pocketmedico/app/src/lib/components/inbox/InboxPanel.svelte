<script lang="ts">
	import { Inbox, CheckCheck } from 'lucide-svelte';
	import InboxItem from './InboxItem.svelte';
	import { EmptyState, Button } from '$lib/components/shared';
	import { userInbox, markAsRead, markAllAsRead, unreadCount } from '$lib/stores/inbox';
	import { goto } from '$app/navigation';

	function handleItemClick(item: { id: string; orderId?: string; read: boolean }) {
		if (!item.read) {
			markAsRead(item.id);
		}
		if (item.orderId) {
			goto(`/orders/${item.orderId}`);
		}
	}
</script>

<div class="rounded-xl border border-gray-200 bg-white">
	<!-- Header -->
	<div class="flex items-center justify-between border-b border-gray-200 px-4 py-3">
		<div class="flex items-center gap-2">
			<Inbox class="h-5 w-5 text-gray-400" />
			<h2 class="font-semibold text-gray-900">Inbox</h2>
			{#if $unreadCount > 0}
				<span class="rounded-full bg-blue-100 px-2 py-0.5 text-xs font-medium text-blue-600">
					{$unreadCount}
				</span>
			{/if}
		</div>
		{#if $unreadCount > 0}
			<button
				onclick={markAllAsRead}
				class="flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700"
			>
				<CheckCheck class="h-4 w-4" />
				Mark all read
			</button>
		{/if}
	</div>

	<!-- Items -->
	<div class="max-h-96 divide-y divide-gray-100 overflow-y-auto">
		{#if $userInbox.length === 0}
			<div class="p-6">
				<EmptyState
					icon={Inbox}
					title="No messages"
					description="You're all caught up!"
				/>
			</div>
		{:else}
			{#each $userInbox as item (item.id)}
				<InboxItem {item} onclick={() => handleItemClick(item)} />
			{/each}
		{/if}
	</div>
</div>
