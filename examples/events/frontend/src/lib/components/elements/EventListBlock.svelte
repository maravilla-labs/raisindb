<script lang="ts">
	import type { ContentElement } from '$lib/types';
	import type { Event } from '$lib/types';
	import { query } from '$lib/raisin';
	import EventCard from '$lib/components/EventCard.svelte';

	let { element }: { element: ContentElement } = $props();

	let events = $state<Event[]>([]);
	let loading = $state(true);

	$effect(() => {
		loadEvents();
	});

	async function loadEvents() {
		const limit = element.limit ? Number(element.limit) : 50;
		const featuredOnly = element.featured_only === true || element.featured_only === 'true';
		const category = element.category_filter ? String(element.category_filter) : '';

		let sql = "SELECT id, path, node_type, properties FROM 'events' WHERE node_type = $1";
		const params: unknown[] = ['events:Event'];

		if (featuredOnly) {
			sql += ` AND properties->>'featured'::String = $${params.length + 1}`;
			params.push('true');
		}

		if (category) {
			sql += ` AND properties->>'category'::String = $${params.length + 1}`;
			params.push(category);
		}

		sql += ` ORDER BY properties->>'start_date'::String ASC LIMIT ${limit}`;

		events = await query<Event>(sql, params);
		loading = false;
	}
</script>

<div class="section event-list-section">
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
			<div class="loading">Loading events...</div>
		{:else if events.length > 0}
			<div class="card-grid">
				{#each events as event}
					<EventCard {event} />
				{/each}
			</div>
		{:else}
			<div class="empty">
				<p>No events scheduled yet.</p>
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
