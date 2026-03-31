<script lang="ts">
	import { Search } from 'lucide-svelte';
	import { goto } from '$app/navigation';

	export let value = '';
	export let placeholder = 'Search articles...';

	function handleSubmit(e: Event) {
		e.preventDefault();
		if (value.trim()) {
			goto(`/search?q=${encodeURIComponent(value.trim())}`);
		}
	}
</script>

<form onsubmit={handleSubmit} class="relative">
	<div class="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3">
		<Search class="h-4 w-4 text-gray-400" />
	</div>
	<input
		type="text"
		bind:value
		{placeholder}
		class="block w-full rounded-lg border border-gray-300 bg-white py-2 pl-10 pr-3 text-sm placeholder-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
	/>
</form>
