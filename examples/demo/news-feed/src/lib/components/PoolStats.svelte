<script lang="ts">
	import { Database } from 'lucide-svelte';
	import { onMount } from 'svelte';

	let stats = $state<{ totalCount: number; idleCount: number; waitingCount: number } | null>(null);

	async function fetchStats() {
		try {
			const res = await fetch('/api/pool-stats');
			stats = await res.json();
		} catch {
			stats = null;
		}
	}

	onMount(() => {
		fetchStats();
		const interval = setInterval(fetchStats, 5000);
		return () => clearInterval(interval);
	});
</script>

{#if stats}
	<div class="fixed bottom-4 right-4 flex items-center gap-2 rounded-lg border border-gray-200 bg-white/90 px-3 py-2 text-xs shadow-md backdrop-blur">
		<Database class="h-3.5 w-3.5 text-gray-500" />
		<span class="font-medium text-gray-700">Pool:</span>
		<span class="text-green-600" title="Total connections">{stats.totalCount}</span>
		<span class="text-gray-400">/</span>
		<span class="text-blue-600" title="Idle connections">{stats.idleCount}</span>
		{#if stats.waitingCount > 0}
			<span class="text-gray-400">/</span>
			<span class="text-amber-600" title="Waiting requests">{stats.waitingCount}</span>
		{/if}
	</div>
{/if}
