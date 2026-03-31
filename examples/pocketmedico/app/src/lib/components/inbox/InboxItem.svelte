<script lang="ts">
	import { Bell, FileAudio, CheckCircle, AlertCircle } from 'lucide-svelte';
	import type { InboxItem, InboxItemType } from '$lib/stores/inbox';
	import { formatRelativeTime } from '$lib/utils/formatters';

	interface Props {
		item: InboxItem;
		onclick?: () => void;
	}

	let { item, onclick }: Props = $props();

	const icons: Record<InboxItemType, typeof Bell> = {
		order_status_update: Bell,
		order_ready: CheckCircle,
		new_review_task: FileAudio,
		review_reminder: AlertCircle
	};

	const iconColors: Record<InboxItemType, string> = {
		order_status_update: 'text-blue-500',
		order_ready: 'text-green-500',
		new_review_task: 'text-orange-500',
		review_reminder: 'text-yellow-500'
	};

	const Icon = icons[item.type];
</script>

<button
	{onclick}
	class="flex w-full items-start gap-3 rounded-lg p-3 text-left transition-colors hover:bg-gray-50
		{item.read ? 'bg-white' : 'bg-blue-50'}"
>
	<div class="mt-0.5 flex-shrink-0">
		<Icon class="h-5 w-5 {iconColors[item.type]}" />
	</div>
	<div class="min-w-0 flex-1">
		<div class="flex items-center gap-2">
			<span class="truncate text-sm font-medium text-gray-900 {!item.read ? 'font-semibold' : ''}">
				{item.title}
			</span>
			{#if !item.read}
				<span class="h-2 w-2 flex-shrink-0 rounded-full bg-blue-500"></span>
			{/if}
		</div>
		<p class="mt-0.5 truncate text-sm text-gray-500">{item.message}</p>
		<p class="mt-1 text-xs text-gray-400">{formatRelativeTime(item.createdAt)}</p>
	</div>
</button>
