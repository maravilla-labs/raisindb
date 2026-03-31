<script lang="ts">
	import type { Page } from '$lib/types';
	import PageRenderer from '$lib/components/PageRenderer.svelte';

	let { page }: { page: Page } = $props();
</script>

<svelte:head>
	<title>{page.properties.title} | Keller Basel</title>
</svelte:head>

<PageRenderer {page} />
