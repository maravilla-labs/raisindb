<script lang="ts">
	import { Clock, AlertTriangle, FileText, User } from 'lucide-svelte';
	import { formatRelativeTime, formatDocumentType } from '$lib/utils/formatters';
	import type { Order } from '$lib/stores/orders';

	interface Props {
		order: Order;
	}

	let { order }: Props = $props();

	// Calculate waiting time
	let waitingTime = $derived(formatRelativeTime(order.createdAt));
</script>

<div class="w-full p-4 text-left">
	<div class="flex items-start justify-between">
		<!-- Left: Order info -->
		<div class="flex-1 min-w-0">
			<div class="flex items-center gap-2">
				<span class="font-semibold text-gray-900">{order.orderNumber}</span>
				{#if order.priority === 'urgent'}
					<span class="inline-flex items-center gap-1 rounded-full bg-red-100 px-2 py-0.5 text-xs font-medium text-red-700">
						<AlertTriangle class="h-3 w-3" />
						Urgent
					</span>
				{/if}
			</div>

			<div class="mt-1 flex items-center gap-3 text-sm text-gray-500">
				<span class="flex items-center gap-1">
					<User class="h-3.5 w-3.5" />
					{order.customerName}
				</span>
				<span class="flex items-center gap-1">
					<FileText class="h-3.5 w-3.5" />
					{formatDocumentType(order.documentType)}
				</span>
			</div>

			<div class="mt-2 text-xs text-gray-400">
				Patient: {order.patientInitials}
			</div>
		</div>

		<!-- Right: Time waiting -->
		<div class="flex flex-col items-end gap-1">
			<div class="flex items-center gap-1 text-sm text-gray-500">
				<Clock class="h-4 w-4" />
				<span>{waitingTime}</span>
			</div>
			{#if order.aiConfidenceScore}
				<div class="text-xs text-gray-400">
					AI Confidence: {order.aiConfidenceScore}%
				</div>
			{/if}
		</div>
	</div>
</div>
