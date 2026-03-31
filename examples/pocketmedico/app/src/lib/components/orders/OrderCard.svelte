<script lang="ts">
	import { FileText, User, Calendar, Sparkles } from 'lucide-svelte';
	import type { Order } from '$lib/stores/orders';
	import { formatDate, formatDocumentType } from '$lib/utils/formatters';
	import StatusBadge from './StatusBadge.svelte';

	interface Props {
		order: Order;
	}

	let { order }: Props = $props();

	const tierLabels = {
		light: 'Light',
		pro: 'Pro'
	};

	const tierColors = {
		light: 'bg-gray-100 text-gray-700',
		pro: 'bg-purple-100 text-purple-700'
	};
</script>

<a
	href="/orders/{order.id}"
	class="block rounded-lg border border-gray-200 bg-white p-4 transition-shadow hover:shadow-md"
>
	<div class="flex items-start justify-between">
		<div class="flex items-center gap-2">
			<FileText class="h-5 w-5 text-gray-400" />
			<span class="font-medium text-gray-900">{order.orderNumber}</span>
		</div>
		<StatusBadge status={order.status} />
	</div>

	<div class="mt-3 flex items-center gap-4 text-sm text-gray-600">
		<div class="flex items-center gap-1.5">
			<User class="h-4 w-4" />
			<span>{order.patientInitials}</span>
		</div>
		<div class="flex items-center gap-1.5">
			<Sparkles class="h-4 w-4" />
			<span class="rounded px-1.5 py-0.5 text-xs font-medium {tierColors[order.tier]}">
				{tierLabels[order.tier]}
			</span>
		</div>
	</div>

	<div class="mt-2 text-sm text-gray-500">
		{formatDocumentType(order.documentType)}
	</div>

	<div class="mt-3 flex items-center gap-1.5 text-xs text-gray-400">
		<Calendar class="h-3.5 w-3.5" />
		<span>{formatDate(order.createdAt)}</span>
	</div>
</a>
