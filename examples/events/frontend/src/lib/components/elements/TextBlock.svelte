<script lang="ts">
	import type { ContentElement } from '$lib/types';

	let { element }: { element: ContentElement } = $props();
</script>

<div class="section text-block">
	<div class="container">
		{#if element.heading}
			<h2 class="text-block-heading">{element.heading}</h2>
		{/if}
		{#if element.body}
			<div class="text-block-body">{element.body}</div>
		{/if}
	</div>
</div>

<style>
	.text-block {
		padding: var(--space-2xl) 0;
	}

	.text-block-heading {
		font-family: var(--font-display);
		font-size: 1.6rem;
		font-weight: 600;
		letter-spacing: -0.03em;
		margin-bottom: var(--space-lg);
		color: var(--color-text);
		position: relative;
		padding-left: var(--space-lg);
	}

	.text-block-heading::before {
		content: '';
		position: absolute;
		left: 0;
		top: 0.2em;
		width: 3px;
		height: 1em;
		background: var(--color-accent);
		box-shadow: 0 0 8px var(--color-accent-glow);
	}

	.text-block-body {
		font-size: 1.05rem;
		line-height: 1.8;
		max-width: 680px;
		color: var(--color-text-secondary);
	}
</style>
