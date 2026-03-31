<script lang="ts">
	import { Plus, FileText } from 'lucide-svelte';
	import { Button, EmptyState } from '$lib/components/shared';
	import { AppHeader } from '$lib/components/layout';
	import { OrderCard } from '$lib/components/orders';
	import { customerOrders } from '$lib/stores/orders';
</script>

<svelte:head>
	<title>Orders - Pocket Medico</title>
</svelte:head>

<AppHeader title="Orders">
	<a href="/orders/new">
		<Button>
			<Plus class="h-4 w-4" />
			New Order
		</Button>
	</a>
</AppHeader>

<div class="p-6">
	{#if $customerOrders.length === 0}
		<EmptyState
			icon={FileText}
			title="No orders yet"
			description="Create your first order to get started with medical transcription."
		>
			<a href="/orders/new">
				<Button>
					<Plus class="h-4 w-4" />
					Create Order
				</Button>
			</a>
		</EmptyState>
	{:else}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
			{#each $customerOrders as order (order.id)}
				<OrderCard {order} />
			{/each}
		</div>
	{/if}
</div>
