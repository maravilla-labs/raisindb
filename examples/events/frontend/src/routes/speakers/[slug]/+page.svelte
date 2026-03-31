<script lang="ts">
	import { page } from '$app/state';
	import { queryOne } from '$lib/raisin';
	import type { Speaker } from '$lib/types';

	let speaker = $state<Speaker | null>(null);
	let loading = $state(true);

	$effect(() => {
		const slug = page.params.slug;
		loading = true;
		queryOne<Speaker>(
			"SELECT id, path, node_type, properties FROM 'events' WHERE node_type = $1 AND properties->>'slug'::String = $2",
			['events:Speaker', slug]
		).then((row) => {
			speaker = row;
			loading = false;
		});
	});
</script>

<svelte:head>
	<title>{speaker?.properties.name ?? 'Artist'} | Keller Basel</title>
</svelte:head>

<div class="container">
	{#if loading}
		<div class="loading">Loading artist...</div>
	{:else if speaker}
		<div class="detail-page">
			<div class="page-header">
				<a href="/speakers">&larr; Back to artists</a>
			</div>

			<div class="detail-hero placeholder" style="display: flex; align-items: center; justify-content: center; font-size: 5rem; font-weight: 700; font-family: var(--font-display); color: var(--color-text-muted); opacity: 0.3; height: 220px;">
				{speaker.properties.name.charAt(0)}
			</div>

			<h1>{speaker.properties.name}</h1>

			{#if speaker.properties.title || speaker.properties.company}
				<div class="artist-role">
					{#if speaker.properties.title}{speaker.properties.title}{/if}
					{#if speaker.properties.title && speaker.properties.company} / {/if}
					{#if speaker.properties.company}{speaker.properties.company}{/if}
				</div>
			{/if}

			{#if speaker.properties.website || speaker.properties.twitter || speaker.properties.linkedin}
				<div class="artist-links">
					{#if speaker.properties.website}
						<a href={speaker.properties.website} target="_blank" rel="noopener noreferrer">Website</a>
					{/if}
					{#if speaker.properties.twitter}
						<a href="https://twitter.com/{speaker.properties.twitter}" target="_blank" rel="noopener noreferrer">Twitter</a>
					{/if}
					{#if speaker.properties.linkedin}
						<a href={speaker.properties.linkedin} target="_blank" rel="noopener noreferrer">LinkedIn</a>
					{/if}
				</div>
			{/if}

			{#if speaker.properties.bio}
				<div class="description">
					<p>{speaker.properties.bio}</p>
				</div>
			{/if}
		</div>
	{:else}
		<div class="empty">
			<p>Artist not found.</p>
			<a href="/speakers">Browse all artists</a>
		</div>
	{/if}
</div>

<style>
	.artist-role {
		font-size: 0.85rem;
		color: var(--color-accent);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		margin-bottom: var(--space-xl);
	}

	.artist-links {
		display: flex;
		gap: var(--space-lg);
		margin-bottom: var(--space-xl);
		padding: var(--space-lg) 0;
		border-top: 1px solid var(--color-border);
		border-bottom: 1px solid var(--color-border);
	}

	.artist-links a {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--color-text-secondary);
		transition: color 0.2s;
	}

	.artist-links a:hover {
		color: var(--color-accent);
	}
</style>
