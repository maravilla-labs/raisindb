<script lang="ts">
	import type { Page } from '$lib/types';
	import { pageComponents, defaultPageComponent } from '$lib/components/pages/index';

	let { data } = $props();
	let page = $derived<Page | null>(data.page);
</script>

<svelte:head>
	<title>Events | Keller Basel</title>
</svelte:head>

<div class="container">
	<div class="page-actions">
		<a href="/events/new" class="create-btn">+ Submit Event</a>
	</div>
</div>

{#if page}
	{@const Component = pageComponents[page.archetype ?? ''] ?? defaultPageComponent}
	<Component {page} />
{:else}
	<div class="container">
		<div class="page-header">
			<h1>Events</h1>
			<p>What's happening in Basel's underground</p>
		</div>
		<div class="empty">
			<p>No events found. Create events in the RaisinDB admin console.</p>
		</div>
	</div>
{/if}

<style>
	.page-actions {
		display: flex;
		justify-content: flex-end;
		padding: var(--space-lg) 0 0;
	}

	.create-btn {
		padding: 0.6rem 1.4rem;
		background: transparent;
		color: var(--color-text-secondary) !important;
		border: 1px solid var(--color-border);
		border-radius: var(--radius);
		font-weight: 500;
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		white-space: nowrap;
		transition: border-color 0.2s, color 0.2s;
	}

	.create-btn:hover {
		border-color: var(--color-accent);
		color: var(--color-accent) !important;
	}
</style>
