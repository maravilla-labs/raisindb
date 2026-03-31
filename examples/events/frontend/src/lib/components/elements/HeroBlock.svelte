<script lang="ts">
	import type { ContentElement } from '$lib/types';

	let { element }: { element: ContentElement } = $props();
</script>

<section class="hero-block">
	<div class="hero-block-bg">
		<div class="hero-block-glow"></div>
	</div>
	<div class="container">
		<div class="hero-block-content">
			{#if element.heading}
				<h1>{element.heading}</h1>
			{/if}
			{#if element.subheading}
				<p class="hero-block-sub">{element.subheading}</p>
			{/if}
			{#if element.cta_text && element.cta_link}
				<a href={String(element.cta_link)} class="hero-block-cta">{element.cta_text}</a>
			{/if}
		</div>
	</div>
</section>

<style>
	.hero-block {
		padding: var(--space-3xl) 0;
		position: relative;
		overflow: hidden;
		border-bottom: 1px solid var(--color-border);
	}

	.hero-block-bg {
		position: absolute;
		inset: 0;
		pointer-events: none;
	}

	.hero-block-glow {
		position: absolute;
		top: -30%;
		right: -10%;
		width: 500px;
		height: 500px;
		background: radial-gradient(circle, rgba(255, 45, 120, 0.15) 0%, transparent 70%);
		filter: blur(80px);
		animation: glowDrift 8s ease-in-out infinite alternate;
	}

	@keyframes glowDrift {
		0% { transform: translate(0, 0); opacity: 0.6; }
		100% { transform: translate(-40px, 20px); opacity: 1; }
	}

	.hero-block-content {
		position: relative;
		z-index: 2;
		max-width: 680px;
	}

	.hero-block h1 {
		font-family: var(--font-display);
		font-size: clamp(2.5rem, 6vw, 4.5rem);
		font-weight: 700;
		letter-spacing: -0.05em;
		line-height: 0.95;
		margin-bottom: var(--space-lg);
	}

	.hero-block-sub {
		font-size: 1.1rem;
		color: var(--color-text-secondary);
		max-width: 480px;
		line-height: 1.7;
	}

	.hero-block-cta {
		display: inline-block;
		margin-top: var(--space-xl);
		padding: 0.85rem 2rem;
		background: var(--color-accent);
		color: white !important;
		font-weight: 600;
		font-size: 0.85rem;
		text-transform: uppercase;
		letter-spacing: 0.1em;
		border-radius: var(--radius);
		transition: background 0.2s, box-shadow 0.3s, transform 0.15s;
	}

	.hero-block-cta:hover {
		background: var(--color-accent-hover);
		box-shadow: 0 0 30px var(--color-accent-glow);
		transform: translateY(-1px);
		color: white !important;
	}
</style>
