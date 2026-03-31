<script lang="ts">
	import { ClipboardList, Clock, CheckCircle, AlertTriangle } from 'lucide-svelte';
	import { Button, EmptyState } from '$lib/components/shared';
	import { AppHeader } from '$lib/components/layout';
	import { InboxPanel } from '$lib/components/inbox';
	import { ReviewQueueCard } from '$lib/components/nurse';
	import { nurseQueue, orders } from '$lib/stores/orders';
	import { currentUser } from '$lib/stores/auth';

	// Stats
	let pendingCount = $derived($nurseQueue.filter((o) => o.status === 'ai_complete').length);
	let inReviewCount = $derived($nurseQueue.filter((o) => o.status === 'in_review').length);
	let completedToday = $derived(
		$orders.filter((o) => {
			if (!o.completedAt || !o.transcript?.humanReviewed) return false;
			const today = new Date();
			return o.completedAt.toDateString() === today.toDateString();
		}).length
	);
	let urgentCount = $derived($nurseQueue.filter((o) => o.priority === 'urgent').length);
</script>

<svelte:head>
	<title>Nurse Dashboard - Pocket Medico</title>
</svelte:head>

<AppHeader title="Review Dashboard" />

<div class="p-6">
	<!-- Welcome -->
	<div class="mb-6">
		<h1 class="text-2xl font-bold text-gray-900">Welcome back, {$currentUser?.displayName}</h1>
		<p class="text-gray-500">Review and approve medical transcriptions.</p>
	</div>

	<!-- Stats -->
	<div class="mb-8 grid gap-4 sm:grid-cols-4">
		<div class="rounded-xl border border-gray-200 bg-white p-5">
			<div class="flex items-center gap-3">
				<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-orange-100">
					<ClipboardList class="h-5 w-5 text-orange-600" />
				</div>
				<div>
					<p class="text-2xl font-bold text-gray-900">{pendingCount}</p>
					<p class="text-sm text-gray-500">Pending Review</p>
				</div>
			</div>
		</div>

		<div class="rounded-xl border border-gray-200 bg-white p-5">
			<div class="flex items-center gap-3">
				<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-blue-100">
					<Clock class="h-5 w-5 text-blue-600" />
				</div>
				<div>
					<p class="text-2xl font-bold text-gray-900">{inReviewCount}</p>
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
					<p class="text-2xl font-bold text-gray-900">{completedToday}</p>
					<p class="text-sm text-gray-500">Completed Today</p>
				</div>
			</div>
		</div>

		<div class="rounded-xl border border-gray-200 bg-white p-5">
			<div class="flex items-center gap-3">
				<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-red-100">
					<AlertTriangle class="h-5 w-5 text-red-600" />
				</div>
				<div>
					<p class="text-2xl font-bold text-gray-900">{urgentCount}</p>
					<p class="text-sm text-gray-500">Urgent</p>
				</div>
			</div>
		</div>
	</div>

	<div class="grid gap-6 lg:grid-cols-2">
		<!-- Review Queue -->
		<div class="rounded-xl border border-gray-200 bg-white">
			<div class="flex items-center justify-between border-b border-gray-200 px-4 py-3">
				<div class="flex items-center gap-2">
					<ClipboardList class="h-5 w-5 text-gray-400" />
					<h2 class="font-semibold text-gray-900">Review Queue</h2>
					{#if $nurseQueue.length > 0}
						<span class="rounded-full bg-orange-100 px-2 py-0.5 text-xs font-medium text-orange-600">
							{$nurseQueue.length}
						</span>
					{/if}
				</div>
			</div>

			<div class="max-h-96 divide-y divide-gray-100 overflow-y-auto">
				{#if $nurseQueue.length === 0}
					<div class="p-6">
						<EmptyState
							icon={ClipboardList}
							title="No pending reviews"
							description="All caught up! Check back later."
						/>
					</div>
				{:else}
					{#each $nurseQueue as order (order.id)}
						<a href="/nurse/review/{order.id}" class="block hover:bg-gray-50">
							<ReviewQueueCard {order} />
						</a>
					{/each}
				{/if}
			</div>
		</div>

		<!-- Inbox -->
		<InboxPanel />
	</div>
</div>
