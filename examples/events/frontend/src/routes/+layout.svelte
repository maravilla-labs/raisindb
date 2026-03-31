<script lang="ts">
	import favicon from '$lib/assets/favicon.svg';
	import { page } from '$app/state';
	import { initSession } from '$lib/raisin';
	import '../app.css';

	let { children } = $props();
	let connected = $state(false);

	$effect(() => {
		initSession().then(() => {
			connected = true;
		});
	});

	function isActive(path: string): boolean {
		return page.url.pathname === path || page.url.pathname.startsWith(path + '/');
	}
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>Keller Basel</title>
</svelte:head>

<nav>
	<div class="container">
		<a href="/" class="logo">Keller<span class="logo-accent">.</span></a>
		<div class="links">
			<a href="/" class:active={page.url.pathname === '/'}>Home</a>
			<a href="/events" class:active={isActive('/events')}>Events</a>
			<a href="/venues" class:active={isActive('/venues')}>Venues</a>
			<a href="/speakers" class:active={isActive('/speakers')}>Artists</a>
		</div>
	</div>
</nav>

{#if connected}
	<main>
		{@render children()}
	</main>
	<footer>
		<div class="container">
			<div class="footer-inner">
				<div class="footer-brand">
					<span class="footer-logo">Keller<span class="logo-accent">.</span></span>
					<span class="footer-tagline">Basel Underground</span>
				</div>
				<div class="footer-meta">
					<span>Basel, CH</span>
					<span class="footer-divider"></span>
					<span>Est. 2024</span>
				</div>
			</div>
		</div>
	</footer>
{:else}
	<div class="connecting">
		<div class="connecting-inner">
			<div class="connecting-spinner"></div>
			<span>Connecting</span>
		</div>
	</div>
{/if}

<style>
	main {
		min-height: calc(100vh - 160px);
	}

	.logo-accent {
		color: var(--color-accent);
	}

	/* Footer */
	footer {
		border-top: 1px solid var(--color-border);
		padding: var(--space-xl) 0;
		margin-top: var(--space-3xl);
	}

	.footer-inner {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	.footer-brand {
		display: flex;
		align-items: baseline;
		gap: var(--space-md);
	}

	.footer-logo {
		font-family: var(--font-display);
		font-size: 1.1rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: -0.03em;
	}

	.footer-tagline {
		font-size: 0.8rem;
		color: var(--color-text-muted);
		text-transform: uppercase;
		letter-spacing: 0.12em;
	}

	.footer-meta {
		display: flex;
		align-items: center;
		gap: var(--space-md);
		font-size: 0.8rem;
		color: var(--color-text-muted);
		text-transform: uppercase;
		letter-spacing: 0.08em;
	}

	.footer-divider {
		width: 3px;
		height: 3px;
		background: var(--color-text-muted);
		border-radius: 50%;
	}

	/* Connecting state */
	.connecting {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: 60vh;
	}

	.connecting-inner {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--space-lg);
		color: var(--color-text-muted);
		font-size: 0.85rem;
		text-transform: uppercase;
		letter-spacing: 0.15em;
	}

	.connecting-spinner {
		width: 32px;
		height: 32px;
		border: 2px solid var(--color-border);
		border-top-color: var(--color-accent);
		border-radius: 50%;
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		to { transform: rotate(360deg); }
	}

	@media (max-width: 768px) {
		.footer-inner {
			flex-direction: column;
			gap: var(--space-md);
			text-align: center;
		}
	}
</style>
