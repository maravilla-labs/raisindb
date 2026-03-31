<script lang="ts">
	import type { Venue } from '$lib/types';

	let { venue }: { venue: Venue } = $props();
</script>

<a href="/venues/{venue.properties.slug}" class="card venue-card">
	<div class="card-image placeholder">
		<span class="card-initial">{venue.properties.title.charAt(0)}</span>
	</div>
	<div class="card-body">
		<h3>{venue.properties.title}</h3>
		{#if venue.properties.description}
			<p>{venue.properties.description.slice(0, 100)}{venue.properties.description.length > 100 ? '...' : ''}</p>
		{/if}
		<div class="card-meta">
			{#if venue.properties.city}
				<span>{venue.properties.city}</span>
			{/if}
			{#if venue.properties.country}
				<span>{venue.properties.country}</span>
			{/if}
			{#if venue.properties.capacity}
				<span class="capacity">{venue.properties.capacity} cap.</span>
			{/if}
		</div>
	</div>
</a>

<style>
	.venue-card .card-image {
		height: 200px;
		background: linear-gradient(
			135deg,
			var(--color-surface-raised) 0%,
			#161820 100%
		);
	}

	.card-initial {
		font-family: var(--font-display);
		font-size: 4rem;
		font-weight: 700;
		color: var(--color-text-muted);
		opacity: 0.35;
		letter-spacing: -0.05em;
	}

	.capacity {
		color: var(--color-accent);
	}
</style>
