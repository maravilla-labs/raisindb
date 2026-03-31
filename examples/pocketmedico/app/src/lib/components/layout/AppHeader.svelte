<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title?: string;
		children?: Snippet;
	}

	let { title, children }: Props = $props();
</script>

<header class="sticky top-0 z-30 flex h-16 items-center justify-between border-b border-gray-200 bg-white/80 px-6 backdrop-blur-md">
	<div class="flex items-center gap-4">
		{#if title}
			<h1 class="text-lg font-semibold text-gray-900">{title}</h1>
		{/if}
	</div>

	<div class="flex items-center gap-3">
		{#if children}
			{@render children()}
		{/if}
	</div>
</header>
