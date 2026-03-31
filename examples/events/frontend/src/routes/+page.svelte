<script lang="ts">
	import type { Page } from '$lib/types';
	import { pageComponents, defaultPageComponent } from '$lib/components/pages/index';

	let { data } = $props();
	let page = $derived<Page | null>(data.page);
</script>

<svelte:head>
	<title>Keller Basel — Underground Events</title>
</svelte:head>

{#if page}
	{@const Component = pageComponents[page.archetype ?? ''] ?? defaultPageComponent}
	<Component {page} />
{:else}
	<section class="landing-hero">
		<div class="hero-bg">
			<div class="hero-grid"></div>
			<div class="hero-glow hero-glow-1"></div>
			<div class="hero-glow hero-glow-2"></div>
		</div>
		<div class="container">
			<div class="hero-content">
				<div class="hero-eyebrow">Basel Underground</div>
				<h1 class="hero-title">
					<span class="hero-line">Where the</span>
					<span class="hero-line hero-line-accent">night</span>
					<span class="hero-line">comes alive</span>
				</h1>
				<p class="hero-subtitle">
					Discover the pulse of Basel's underground scene. From techno to experimental, we curate nights that move you.
				</p>
				<div class="hero-actions">
					<a href="/events" class="hero-btn-primary">Explore Events</a>
					<a href="/venues" class="hero-btn-secondary">Our Venues</a>
				</div>
			</div>
		</div>
		<div class="hero-scroll-indicator">
			<span>Scroll</span>
			<div class="scroll-line"></div>
		</div>
	</section>
{/if}

<style>
	.landing-hero {
		min-height: 92vh;
		display: flex;
		align-items: center;
		position: relative;
		overflow: hidden;
	}

	.hero-bg {
		position: absolute;
		inset: 0;
		pointer-events: none;
	}

	.hero-grid {
		position: absolute;
		inset: 0;
		background-image:
			linear-gradient(rgba(255, 255, 255, 0.02) 1px, transparent 1px),
			linear-gradient(90deg, rgba(255, 255, 255, 0.02) 1px, transparent 1px);
		background-size: 80px 80px;
		mask-image: radial-gradient(ellipse 80% 60% at 50% 40%, black 30%, transparent 100%);
		-webkit-mask-image: radial-gradient(ellipse 80% 60% at 50% 40%, black 30%, transparent 100%);
	}

	.hero-glow {
		position: absolute;
		border-radius: 50%;
		filter: blur(100px);
	}

	.hero-glow-1 {
		top: 10%;
		right: -5%;
		width: 500px;
		height: 500px;
		background: radial-gradient(circle, rgba(255, 45, 120, 0.2) 0%, transparent 70%);
		animation: float1 10s ease-in-out infinite alternate;
	}

	.hero-glow-2 {
		bottom: -10%;
		left: 10%;
		width: 400px;
		height: 400px;
		background: radial-gradient(circle, rgba(255, 45, 120, 0.08) 0%, transparent 70%);
		animation: float2 12s ease-in-out infinite alternate;
	}

	@keyframes float1 {
		0% { transform: translate(0, 0) scale(1); }
		100% { transform: translate(-60px, 30px) scale(1.15); }
	}

	@keyframes float2 {
		0% { transform: translate(0, 0) scale(1); }
		100% { transform: translate(40px, -20px) scale(1.1); }
	}

	.hero-content {
		position: relative;
		z-index: 2;
		max-width: 720px;
		animation: fadeUp 0.8s ease-out;
	}

	@keyframes fadeUp {
		from {
			opacity: 0;
			transform: translateY(30px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	.hero-eyebrow {
		font-size: 0.75rem;
		text-transform: uppercase;
		letter-spacing: 0.25em;
		color: var(--color-accent);
		margin-bottom: var(--space-xl);
		font-weight: 500;
	}

	.hero-title {
		font-family: var(--font-display);
		font-weight: 700;
		line-height: 0.9;
		letter-spacing: -0.05em;
		margin-bottom: var(--space-xl);
	}

	.hero-line {
		display: block;
		font-size: clamp(3rem, 9vw, 6.5rem);
	}

	.hero-line-accent {
		color: var(--color-accent);
		font-style: italic;
		text-shadow: 0 0 60px var(--color-accent-glow);
	}

	.hero-subtitle {
		font-size: 1.1rem;
		color: var(--color-text-secondary);
		max-width: 460px;
		line-height: 1.7;
		margin-bottom: var(--space-xl);
	}

	.hero-actions {
		display: flex;
		gap: var(--space-md);
		align-items: center;
	}

	.hero-btn-primary {
		padding: 0.9rem 2.2rem;
		background: var(--color-accent);
		color: white !important;
		font-weight: 600;
		font-size: 0.85rem;
		text-transform: uppercase;
		letter-spacing: 0.1em;
		border-radius: var(--radius);
		transition: background 0.2s, box-shadow 0.3s, transform 0.15s;
	}

	.hero-btn-primary:hover {
		background: var(--color-accent-hover);
		box-shadow: 0 0 40px var(--color-accent-glow);
		transform: translateY(-1px);
		color: white !important;
	}

	.hero-btn-secondary {
		padding: 0.9rem 2.2rem;
		color: var(--color-text-secondary) !important;
		font-weight: 500;
		font-size: 0.85rem;
		text-transform: uppercase;
		letter-spacing: 0.1em;
		border: 1px solid var(--color-border);
		border-radius: var(--radius);
		transition: border-color 0.2s, color 0.2s;
	}

	.hero-btn-secondary:hover {
		border-color: var(--color-text-muted);
		color: var(--color-text) !important;
	}

	/* Scroll indicator */
	.hero-scroll-indicator {
		position: absolute;
		bottom: 2.5rem;
		right: var(--space-lg);
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--space-sm);
		color: var(--color-text-muted);
		font-size: 0.65rem;
		text-transform: uppercase;
		letter-spacing: 0.2em;
		animation: fadeUp 0.8s ease-out 0.4s both;
	}

	.scroll-line {
		width: 1px;
		height: 40px;
		background: linear-gradient(to bottom, var(--color-text-muted), transparent);
		animation: scrollPulse 2s ease-in-out infinite;
	}

	@keyframes scrollPulse {
		0%, 100% { opacity: 0.3; }
		50% { opacity: 1; }
	}

	@media (max-width: 768px) {
		.landing-hero {
			min-height: 85vh;
		}

		.hero-actions {
			flex-direction: column;
			align-items: flex-start;
		}

		.hero-scroll-indicator {
			display: none;
		}
	}
</style>
