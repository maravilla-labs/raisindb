<script lang="ts">
	export let tag: string;
	export let href: string | null = null;
</script>

{#if href}
	<a
		{href}
		class="inline-flex items-center rounded-full bg-gray-100 px-2.5 py-0.5 text-xs font-medium text-gray-800 transition-colors hover:bg-gray-200"
	>
		{tag}
	</a>
{:else}
	<span
		class="inline-flex items-center rounded-full bg-gray-100 px-2.5 py-0.5 text-xs font-medium text-gray-800"
	>
		{tag}
	</span>
{/if}
