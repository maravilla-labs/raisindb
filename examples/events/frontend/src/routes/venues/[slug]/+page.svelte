<script lang="ts">
	import { page } from '$app/state';
	import { queryOne } from '$lib/raisin';
	import type { Venue } from '$lib/types';

	let venue = $state<Venue | null>(null);
	let loading = $state(true);

	$effect(() => {
		const slug = page.params.slug;
		loading = true;
		queryOne<Venue>(
			"SELECT id, path, node_type, properties FROM 'events' WHERE node_type = $1 AND properties->>'slug'::String = $2",
			['events:Venue', slug]
		).then((row) => {
			venue = row;
			loading = false;
		});
	});
</script>

<svelte:head>
	<title>{venue?.properties.title ?? 'Venue'} | Keller Basel</title>
</svelte:head>

<div class="container">
	{#if loading}
		<div class="loading">Loading venue...</div>
	{:else if venue}
		<div class="detail-page">
			<div class="page-header">
				<a href="/venues">&larr; Back to venues</a>
			</div>

			<div class="detail-hero placeholder" style="display: flex; align-items: center; justify-content: center; font-size: 5rem; font-weight: 700; font-family: var(--font-display); color: var(--color-text-muted); opacity: 0.3;">
				{venue.properties.title.charAt(0)}
			</div>

			<h1>{venue.properties.title}</h1>

			<div class="meta-grid">
				{#if venue.properties.address}
					<div class="meta-item">
						<span class="meta-label">Address</span>
						<span class="meta-value">{venue.properties.address}</span>
					</div>
				{/if}
				{#if venue.properties.city}
					<div class="meta-item">
						<span class="meta-label">City</span>
						<span class="meta-value">{venue.properties.city}</span>
					</div>
				{/if}
				{#if venue.properties.country}
					<div class="meta-item">
						<span class="meta-label">Country</span>
						<span class="meta-value">{venue.properties.country}</span>
					</div>
				{/if}
				{#if venue.properties.capacity}
					<div class="meta-item">
						<span class="meta-label">Capacity</span>
						<span class="meta-value">{venue.properties.capacity}</span>
					</div>
				{/if}
			</div>

			{#if venue.properties.website || venue.properties.contact_email}
				<div class="venue-links">
					{#if venue.properties.website}
						<a href={venue.properties.website} target="_blank" rel="noopener noreferrer">Website</a>
					{/if}
					{#if venue.properties.contact_email}
						<a href="mailto:{venue.properties.contact_email}">Contact</a>
					{/if}
				</div>
			{/if}

			{#if venue.properties.description}
				<div class="description">
					<p>{venue.properties.description}</p>
				</div>
			{/if}
		</div>
	{:else}
		<div class="empty">
			<p>Venue not found.</p>
			<a href="/venues">Browse all venues</a>
		</div>
	{/if}
</div>

<style>
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

	.venue-links {
		display: flex;
		gap: var(--space-lg);
		margin-bottom: var(--space-xl);
	}

	.venue-links a {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--color-text-secondary);
		transition: color 0.2s;
	}

	.venue-links a:hover {
		color: var(--color-accent);
	}
</style>
