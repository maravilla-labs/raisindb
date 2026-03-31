<script lang="ts">
	import { Plus, FileAudio, Clock, CheckCircle, Download } from 'lucide-svelte';
	import { Button } from '$lib/components/shared';
	import { AppHeader } from '$lib/components/layout';
	import { InboxPanel } from '$lib/components/inbox';
	import { customerOrders } from '$lib/stores/orders';
	import { currentUser } from '$lib/stores/auth';
	import { formatRelativeTime } from '$lib/utils/formatters';

	// Stats
	let pendingCount = $derived($customerOrders.filter((o) => o.status === 'pending' || o.status === 'processing').length);
	let readyCount = $derived($customerOrders.filter((o) => o.status === 'ready').length);
	let completedCount = $derived($customerOrders.filter((o) => o.status === 'downloaded').length);

	// Recent orders (last 5)
	let recentOrders = $derived($customerOrders.slice(0, 5));
</script>

<svelte:head>
	<title>Dashboard - Pocket Medico</title>
</svelte:head>

<AppHeader title="Dashboard">
	<a href="/orders/new">
		<Button>
			<Plus class="h-4 w-4" />
			New Order
		</Button>
	</a>
</AppHeader>

<div class="p-6">
	<!-- Welcome -->
	<div class="mb-6">
		<h1 class="text-2xl font-bold text-gray-900">Welcome back, {$currentUser?.displayName}</h1>
		<p class="text-gray-500">Here's what's happening with your transcriptions.</p>
	</div>

	<!-- Stats -->
	<div class="mb-8 grid gap-4 sm:grid-cols-3">
		<div class="rounded-xl border border-gray-200 bg-white p-5">
			<div class="flex items-center gap-3">
				<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-yellow-100">
					<Clock class="h-5 w-5 text-yellow-600" />
				</div>
				<div>
					<p class="text-2xl font-bold text-gray-900">{pendingCount}</p>
					<p class="text-sm text-gray-500">In Progress</p>
				</div>
			</div>
		</div>

		<div class="rounded-xl border border-gray-200 bg-white p-5">
			<div class="flex items-center gap-3">
				<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-green-100">
					<CheckCircle class="h-5 w-5 text-green-600" />
				</div>
				<div>
					<p class="text-2xl font-bold text-gray-900">{readyCount}</p>
					<p class="text-sm text-gray-500">Ready to Download</p>
				</div>
			</div>
		</div>

		<div class="rounded-xl border border-gray-200 bg-white p-5">
			<div class="flex items-center gap-3">
				<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-blue-100">
					<Download class="h-5 w-5 text-blue-600" />
				</div>
				<div>
					<p class="text-2xl font-bold text-gray-900">{completedCount}</p>
					<p class="text-sm text-gray-500">Completed</p>
				</div>
			</div>
		</div>
	</div>

	<div class="grid gap-6 lg:grid-cols-2">
		<!-- Recent Orders -->
		<div class="rounded-xl border border-gray-200 bg-white">
			<div class="flex items-center justify-between border-b border-gray-200 px-4 py-3">
				<div class="flex items-center gap-2">
					<FileAudio class="h-5 w-5 text-gray-400" />
					<h2 class="font-semibold text-gray-900">Recent Orders</h2>
				</div>
				<a href="/orders" class="text-sm text-blue-600 hover:text-blue-700">View all</a>
			</div>

			<div class="divide-y divide-gray-100">
				{#if recentOrders.length === 0}
					<div class="p-6 text-center text-sm text-gray-500">
						No orders yet. <a href="/orders/new" class="text-blue-600 hover:text-blue-700">Create your first order</a>
					</div>
				{:else}
					{#each recentOrders as order (order.id)}
						<a
							href="/orders/{order.id}"
							class="flex items-center justify-between px-4 py-3 hover:bg-gray-50"
						>
							<div>
								<p class="text-sm font-medium text-gray-900">{order.orderNumber}</p>
								<p class="text-xs text-gray-500">Patient: {order.patientInitials}</p>
							</div>
							<div class="text-right">
								<span
									class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium
										{order.status === 'ready'
										? 'bg-green-100 text-green-700'
										: order.status === 'processing'
											? 'bg-yellow-100 text-yellow-700'
											: order.status === 'error'
												? 'bg-red-100 text-red-700'
												: 'bg-gray-100 text-gray-700'}"
								>
									{order.status === 'ai_complete' ? 'In Review' : order.status}
								</span>
								<p class="mt-1 text-xs text-gray-400">{formatRelativeTime(order.createdAt)}</p>
							</div>
						</a>
					{/each}
				{/if}
			</div>
		</div>

		<!-- Inbox -->
		<InboxPanel />
	</div>
</div>
