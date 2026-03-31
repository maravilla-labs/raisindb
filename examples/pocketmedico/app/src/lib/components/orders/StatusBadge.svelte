<script lang="ts">
	import type { OrderStatus } from '$lib/stores/orders';
	import { formatOrderStatus } from '$lib/utils/formatters';

	interface Props {
		status: OrderStatus;
	}

	let { status }: Props = $props();

	const statusColors: Record<OrderStatus, string> = {
		ready: 'bg-green-100 text-green-800',
		downloaded: 'bg-green-100 text-green-800',
		processing: 'bg-yellow-100 text-yellow-800',
		error: 'bg-red-100 text-red-800',
		pending: 'bg-gray-100 text-gray-800',
		ai_complete: 'bg-blue-100 text-blue-800',
		in_review: 'bg-blue-100 text-blue-800'
	};
</script>

<span
	class="inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium {statusColors[status]}"
>
	{formatOrderStatus(status)}
</span>
