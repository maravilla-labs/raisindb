<script lang="ts">
	import type { Page } from '$lib/types';
	import { pageComponents, defaultPageComponent } from '$lib/components/pages/index';

	let { data } = $props();
	let page = $derived<Page | null>(data.page);
</script>

<svelte:head>
	<title>Artists | Keller Basel</title>
</svelte:head>

{#if page}
	{@const Component = pageComponents[page.archetype ?? ''] ?? defaultPageComponent}
	<Component {page} />
{:else}
	<div class="container">
		<div class="page-header">
			<h1>Artists</h1>
			<p>The people behind the sound</p>
		</div>
		<div class="empty">
			<p>No artists announced yet.</p>
		</div>
	</div>
{/if}
