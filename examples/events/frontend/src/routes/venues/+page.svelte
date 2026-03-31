<script lang="ts">
	import type { Page } from '$lib/types';
	import { pageComponents, defaultPageComponent } from '$lib/components/pages/index';

	let { data } = $props();
	let page = $derived<Page | null>(data.page);
</script>

<svelte:head>
	<title>Venues | Keller Basel</title>
</svelte:head>

{#if page}
	{@const Component = pageComponents[page.archetype ?? ''] ?? defaultPageComponent}
	<Component {page} />
{:else}
	<div class="container">
		<div class="page-header">
			<h1>Venues</h1>
			<p>Where it all happens</p>
		</div>
		<div class="empty">
			<p>No venues listed yet.</p>
		</div>
	</div>
{/if}
