<script lang="ts">
	import type { Speaker } from '$lib/types';

	let { speaker }: { speaker: Speaker } = $props();
</script>

<a href="/speakers/{speaker.properties.slug}" class="card speaker-card">
	<div class="card-image placeholder" style="height: 160px;">
		<span class="card-initial">{speaker.properties.name.charAt(0)}</span>
	</div>
	<div class="card-body">
		<h3>{speaker.properties.name}</h3>
		{#if speaker.properties.title || speaker.properties.company}
			<p class="speaker-role">
				{#if speaker.properties.title}{speaker.properties.title}{/if}
				{#if speaker.properties.title && speaker.properties.company} / {/if}
				{#if speaker.properties.company}{speaker.properties.company}{/if}
			</p>
		{/if}
		{#if speaker.properties.bio}
			<p class="speaker-bio">{speaker.properties.bio.slice(0, 90)}{speaker.properties.bio.length > 90 ? '...' : ''}</p>
		{/if}
	</div>
</a>

<style>
	.speaker-card .card-image {
		background: linear-gradient(
			160deg,
			var(--color-surface-raised) 0%,
			#1a1520 100%
		);
	}

	.card-initial {
		font-family: var(--font-display);
		font-size: 3.5rem;
		font-weight: 700;
		color: var(--color-text-muted);
		opacity: 0.35;
		letter-spacing: -0.05em;
	}

	.speaker-role {
		font-size: 0.8rem !important;
		color: var(--color-accent) !important;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		margin-bottom: var(--space-sm);
	}

	.speaker-bio {
		margin-top: var(--space-sm);
	}
</style>
