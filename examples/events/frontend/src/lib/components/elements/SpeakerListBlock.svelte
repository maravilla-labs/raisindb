<script lang="ts">
	import type { ContentElement } from '$lib/types';
	import type { Speaker } from '$lib/types';
	import { query } from '$lib/raisin';
	import SpeakerCard from '$lib/components/SpeakerCard.svelte';

	let { element }: { element: ContentElement } = $props();

	let speakers = $state<Speaker[]>([]);
	let loading = $state(true);

	$effect(() => {
		loadSpeakers();
	});

	async function loadSpeakers() {
		const limit = element.limit ? Number(element.limit) : 50;

		const sql = `SELECT id, path, node_type, properties FROM 'events' WHERE node_type = $1 ORDER BY properties->>'name'::String ASC LIMIT ${limit}`;

		speakers = await query<Speaker>(sql, ['events:Speaker']);
		loading = false;
	}
</script>

<div class="section speaker-list-section">
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
			<div class="loading">Loading artists...</div>
		{:else if speakers.length > 0}
			<div class="card-grid">
				{#each speakers as speaker}
					<SpeakerCard {speaker} />
				{/each}
			</div>
		{:else}
			<div class="empty">
				<p>No artists announced yet.</p>
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
