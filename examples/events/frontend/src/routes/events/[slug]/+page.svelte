<script lang="ts">
	import { page } from '$app/state';
	import { queryOne } from '$lib/raisin';
	import type { Event } from '$lib/types';

	let event = $state<Event | null>(null);
	let loading = $state(true);

	$effect(() => {
		const slug = page.params.slug;
		loading = true;
		queryOne<Event>(
			"SELECT id, path, node_type, properties FROM 'events' WHERE node_type = $1 AND properties->>'slug'::String = $2",
			['events:Event', slug]
		).then((row) => {
			event = row;
			loading = false;
		});
	});

	function formatDate(dateStr: string): string {
		try {
			return new Date(dateStr).toLocaleDateString('en-US', {
				weekday: 'short',
				month: 'long',
				day: 'numeric',
				year: 'numeric',
			});
		} catch {
			return dateStr;
		}
	}

	function formatTime(dateStr: string): string {
		try {
			return new Date(dateStr).toLocaleTimeString('en-US', {
				hour: '2-digit',
				minute: '2-digit',
				hour12: false,
			});
		} catch {
			return '';
		}
	}

	function formatPrice(price: number, currency: string): string {
		if (price === 0) return 'Free';
		return new Intl.NumberFormat('de-CH', { style: 'currency', currency }).format(price);
	}
</script>

<svelte:head>
	<title>{event?.properties.title ?? 'Event'} | Keller Basel</title>
</svelte:head>

<div class="container">
	{#if loading}
		<div class="loading">Loading event...</div>
	{:else if event}
		<div class="detail-page">
			<div class="page-header">
				<a href="/events">&larr; Back to events</a>
			</div>

			<div class="detail-hero placeholder" style="display: flex; align-items: center; justify-content: center; font-size: 5rem; font-weight: 700; font-family: var(--font-display); color: var(--color-text-muted); opacity: 0.3;">
				{event.properties.title.charAt(0)}
			</div>

			<div class="event-header">
				<div class="event-date-badge">
					<span class="date-label">{formatDate(event.properties.start_date)}</span>
					<span class="time-label">{formatTime(event.properties.start_date)}</span>
				</div>
				<h1>{event.properties.title}</h1>
			</div>

			<div class="meta-grid">
				{#if event.properties.location}
					<div class="meta-item">
						<span class="meta-label">Location</span>
						<span class="meta-value">{event.properties.location}</span>
					</div>
				{/if}
				{#if event.properties.end_date}
					<div class="meta-item">
						<span class="meta-label">Until</span>
						<span class="meta-value">{formatDate(event.properties.end_date)}</span>
					</div>
				{/if}
				{#if event.properties.price !== undefined}
					<div class="meta-item">
						<span class="meta-label">Entry</span>
						<span class="meta-value">{formatPrice(event.properties.price, event.properties.currency ?? 'CHF')}</span>
					</div>
				{/if}
				{#if event.properties.capacity}
					<div class="meta-item">
						<span class="meta-label">Capacity</span>
						<span class="meta-value">{event.properties.capacity}</span>
					</div>
				{/if}
			</div>

			{#if event.properties.category || event.properties.featured}
				<div class="badges-row">
					{#if event.properties.category}
						<span class="badge">{event.properties.category}</span>
					{/if}
					{#if event.properties.featured}
						<span class="badge green">Featured</span>
					{/if}
				</div>
			{/if}

			{#if event.properties.tags && event.properties.tags.length > 0}
				<div class="tags-row">
					{#each event.properties.tags as tag}
						<span class="badge outline">{tag}</span>
					{/each}
				</div>
			{/if}

			{#if event.properties.description}
				<div class="description">
					<p>{event.properties.description}</p>
				</div>
			{/if}
		</div>
	{:else}
		<div class="empty">
			<p>Event not found.</p>
			<a href="/events">Browse all events</a>
		</div>
	{/if}
</div>

<style>
	.event-header {
		margin-bottom: var(--space-xl);
	}

	.event-date-badge {
		display: flex;
		align-items: baseline;
		gap: var(--space-md);
		margin-bottom: var(--space-md);
	}

	.date-label {
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--color-accent);
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}

	.time-label {
		font-size: 0.8rem;
		color: var(--color-text-muted);
		letter-spacing: 0.04em;
	}

	.meta-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
		gap: var(--space-lg);
		padding: var(--space-xl) 0;
		border-top: 1px solid var(--color-border);
		border-bottom: 1px solid var(--color-border);
		margin-bottom: var(--space-xl);
	}

	.meta-item {
		display: flex;
		flex-direction: column;
		gap: var(--space-xs);
	}

	.meta-label {
		font-size: 0.7rem;
		text-transform: uppercase;
		letter-spacing: 0.12em;
		color: var(--color-text-muted);
	}

	.meta-value {
		font-size: 0.95rem;
		color: var(--color-text);
		font-weight: 500;
	}

	.badges-row {
		display: flex;
		gap: var(--space-sm);
		margin-bottom: var(--space-lg);
	}

	.tags-row {
		display: flex;
		gap: var(--space-sm);
		flex-wrap: wrap;
		margin-bottom: var(--space-xl);
	}
</style>
