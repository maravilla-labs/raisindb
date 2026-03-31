<script lang="ts">
	import type { ContentElement } from '$lib/types';

	let { element }: { element: ContentElement } = $props();
</script>

<div class="section schedule-section">
	<div class="container">
		<div class="section-header">
			{#if element.heading}
				<h2 class="section-title">{element.heading}</h2>
			{/if}
			{#if element.description}
				<p class="section-description">{element.description}</p>
			{/if}
		</div>
		<div class="schedule-placeholder">
			<div class="schedule-line"></div>
			<p>Schedule coming soon</p>
		</div>
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

	.schedule-placeholder {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--space-lg);
		padding: var(--space-3xl) 0;
		color: var(--color-text-muted);
		font-size: 0.85rem;
		text-transform: uppercase;
		letter-spacing: 0.1em;
	}

	.schedule-line {
		width: 40px;
		height: 1px;
		background: var(--color-border);
	}
</style>
