<script lang="ts">
	import type { Event } from '$lib/types';

	let { event }: { event: Event } = $props();

	function formatDate(dateStr: string): string {
		try {
			const d = new Date(dateStr);
			return d.toLocaleDateString('en-US', {
				month: 'short',
				day: 'numeric',
			}).toUpperCase();
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
</script>

<a href="/events/{event.properties.slug}" class="card event-card">
	<div class="card-image placeholder">
		<span class="card-initial">{event.properties.title.charAt(0)}</span>
		{#if event.properties.featured}
			<span class="featured-dot"></span>
		{/if}
	</div>
	<div class="card-body">
		<div class="card-date">
			<span class="date-text">{formatDate(event.properties.start_date)}</span>
			<span class="time-text">{formatTime(event.properties.start_date)}</span>
		</div>
		<h3>{event.properties.title}</h3>
		{#if event.properties.description}
			<p>{event.properties.description.slice(0, 100)}{event.properties.description.length > 100 ? '...' : ''}</p>
		{/if}
		<div class="card-meta">
			{#if event.properties.location}
				<span>{event.properties.location}</span>
			{/if}
			{#if event.properties.category}
				<span class="badge outline">{event.properties.category}</span>
			{/if}
		</div>
	</div>
</a>

<style>
	.event-card .card-image {
		position: relative;
		height: 180px;
		background: linear-gradient(
			145deg,
			var(--color-surface-raised) 0%,
			var(--color-border) 100%
		);
	}

	.card-initial {
		font-family: var(--font-display);
		font-size: 4rem;
		font-weight: 700;
		color: var(--color-text-muted);
		opacity: 0.4;
		letter-spacing: -0.05em;
	}

	.featured-dot {
		position: absolute;
		top: var(--space-md);
		right: var(--space-md);
		width: 8px;
		height: 8px;
		background: var(--color-accent);
		border-radius: 50%;
		box-shadow: 0 0 12px var(--color-accent-glow);
	}

	.card-date {
		display: flex;
		align-items: baseline;
		gap: var(--space-sm);
		margin-bottom: var(--space-sm);
	}

	.date-text {
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--color-accent);
		letter-spacing: 0.06em;
	}

	.time-text {
		font-size: 0.7rem;
		color: var(--color-text-muted);
		letter-spacing: 0.04em;
	}
</style>
