<script lang="ts">
	import type { ContentElement } from '$lib/types';
	import type { Venue } from '$lib/types';
	import { query } from '$lib/raisin';
	import VenueCard from '$lib/components/VenueCard.svelte';

	let { element }: { element: ContentElement } = $props();

	let venues = $state<Venue[]>([]);
	let loading = $state(true);

	$effect(() => {
		loadVenues();
	});

	async function loadVenues() {
		const limit = element.limit ? Number(element.limit) : 50;

		const sql = `SELECT id, path, node_type, properties FROM 'events' WHERE node_type = $1 ORDER BY properties->>'title'::String ASC LIMIT ${limit}`;

		venues = await query<Venue>(sql, ['events:Venue']);
		loading = false;
	}
</script>

<div class="section venue-list-section">
	<div class="container">
		<div class="section-header">
			{#if element.heading}
				<h2 class="section-title">{element.heading}</h2>
			{/if}
			{#if element.description}
				<p class="section-description">{element.description}</p>
			{/if}
		</div>

		{#if loading}
			<div class="loading">Loading venues...</div>
		{:else if venues.length > 0}
			<div class="card-grid">
				{#each venues as venue}
					<VenueCard {venue} />
				{/each}
			</div>
		{:else}
			<div class="empty">
				<p>No venues listed yet.</p>
			</div>
		{/if}
	</div>
</div>

<style>
	.section-header {
		margin-bottom: var(--space-xl);
	}

	.section-description {
		color: var(--color-text-secondary);
		font-size: 1rem;
		max-width: 500px;
	}
</style>
