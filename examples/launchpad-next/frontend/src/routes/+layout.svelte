<script lang="ts">
  import { page } from '$app/stores';
  import { goto, invalidateAll } from '$app/navigation';
  import { onMount } from 'svelte';
  import { type NavItem } from '$lib/raisin';
  import { setNavigation, setCurrentPath } from '$lib/stores/navigation';
  import { auth, user } from '$lib/stores/auth';
  import { connection, connected } from '$lib/stores/connection';
  import { notificationStore } from '$lib/stores/notifications';
  import NotificationBell from '$lib/components/NotificationBell.svelte';
  import VoiceActivation from '$lib/components/VoiceActivation.svelte';
  import ToastContainer from '$lib/components/ToastContainer.svelte';
  import LanguageSwitcher from '$lib/components/LanguageSwitcher.svelte';
  import type { LayoutData } from './$types';
  import '../app.css';

  interface Props {
    data: LayoutData;
    children: import('svelte').Snippet;
  }

  let { data, children }: Props = $props();
  let mobileMenuOpen = $state(false);
  let scrolled = $state(false);

  // Store initialization and cleanup
  $effect(() => {
    const userObj = $user;
    if (userObj?.home) {
      notificationStore.init();
    } else {
      notificationStore.reset();
    }
  });

  onMount(() => {
    connection.init();

    const handleScroll = () => {
      scrolled = window.scrollY > 20;
    };
    window.addEventListener('scroll', handleScroll, { passive: true });

    return () => {
      connection.cleanup();
      window.removeEventListener('scroll', handleScroll);
    };
  });

  $effect(() => {
    if (data.navigationItems) {
      setNavigation(data.navigationItems as NavItem[]);
    }
    if (data.user !== undefined) {
      auth.setUser(data.user);
    }
  });

  $effect(() => {
    setCurrentPath($page.url.pathname);
  });

  // Close mobile menu on navigation
  $effect(() => {
    $page.url.pathname;
    mobileMenuOpen = false;
  });

  async function handleLogout() {
    await auth.logout();
    await invalidateAll();
    goto('/');
  }
</script>

<div class="app">
  <header class="header" class:scrolled>
    <nav class="nav">
      <a href="/" class="logo">
        <span class="logo-mark">
          <span class="connection-dot" class:connected={$connected}></span>
        </span>
        <span class="logo-text">Nachtkultur</span>
      </a>

      {#if !data.error}
        <button
          class="mobile-toggle"
          onclick={() => mobileMenuOpen = !mobileMenuOpen}
          aria-label="Toggle menu"
        >
          <span class="toggle-bar" class:open={mobileMenuOpen}></span>
          <span class="toggle-bar" class:open={mobileMenuOpen}></span>
        </button>

        <ul class="nav-links" class:open={mobileMenuOpen}>
          {#each data.navigationItems as item}
            {@const slug = item.properties.slug || item.path || item.name}
            <li>
              <a
                href="/{slug}"
                class:active={$page.url.pathname === `/${slug}` || $page.url.pathname.startsWith(`/${slug}/`)}
              >
                {item.properties.title || item.name}
              </a>
            </li>
          {/each}
          <li>
            <a
              href="/kanban"
              class:active={$page.url.pathname === '/kanban'}
            >
              Kanban
            </a>
          </li>
        </ul>
      {/if}

      <div class="auth-section">
        <LanguageSwitcher />
        {#if $user}
          <VoiceActivation />
          <NotificationBell />
          <a href="/profile" class="user-info-link">
            <div class="user-avatar">
              {($user.displayName || $user.email || '?').charAt(0).toUpperCase()}
            </div>
          </a>
          <button class="btn-logout" onclick={handleLogout}>Logout</button>
        {:else}
          <a href="/auth/login" class="btn-login">Sign In</a>
          <a href="/auth/register" class="btn-register">Join</a>
        {/if}
      </div>
    </nav>
  </header>

  <main class="main">
    {#if data.error}
      <div class="error-container">
        <div class="error-icon">!</div>
        <h1>Connection Error</h1>
        <p>{data.error}</p>
        <p class="hint">Make sure RaisinDB is running on 192.168.1.180:8081</p>
      </div>
    {:else}
      {@render children()}
    {/if}
  </main>

  <footer class="footer">
    <div class="footer-inner">
      <div class="footer-brand">
        <span class="footer-logo">Nachtkultur</span>
        <span class="footer-divider"></span>
        <span class="footer-location">Basel, CH</span>
      </div>
      <p class="footer-credit">Powered by RaisinDB</p>
    </div>
  </footer>
</div>

<!-- Toast notifications -->
<ToastContainer />

<style>
  .app {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
  }

  /* ---- Header ---- */
  .header {
    position: sticky;
    top: 0;
    z-index: 100;
    background: rgba(10, 10, 11, 0.8);
    backdrop-filter: blur(var(--glass-blur));
    -webkit-backdrop-filter: blur(var(--glass-blur));
    border-bottom: 1px solid transparent;
    transition: border-color 0.3s ease, background 0.3s ease;
  }

  .header.scrolled {
    border-bottom-color: var(--color-border-subtle);
    background: rgba(10, 10, 11, 0.95);
  }

  .nav {
    max-width: 1200px;
    margin: 0 auto;
    padding: 1rem 2rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 2rem;
  }

  /* Logo */
  .logo {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    text-decoration: none;
    color: var(--color-text-heading);
  }

  .logo-mark {
    width: 32px;
    height: 32px;
    border: 1.5px solid var(--color-accent);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
  }

  .connection-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-error);
    transition: background 0.3s, box-shadow 0.3s;
  }

  .connection-dot.connected {
    background: var(--color-success);
    box-shadow: 0 0 8px rgba(62, 207, 142, 0.4);
  }

  .logo-text {
    font-family: var(--font-display);
    font-size: 1.25rem;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--color-text-heading);
  }

  /* Mobile toggle */
  .mobile-toggle {
    display: none;
    flex-direction: column;
    gap: 5px;
    background: none;
    border: none;
    cursor: pointer;
    padding: 4px;
  }

  .toggle-bar {
    width: 22px;
    height: 1.5px;
    background: var(--color-text);
    transition: transform 0.3s, opacity 0.3s;
  }

  .toggle-bar.open:first-child {
    transform: rotate(45deg) translate(2px, 2px);
  }

  .toggle-bar.open:last-child {
    transform: rotate(-45deg) translate(2px, -2px);
  }

  /* Nav links */
  .nav-links {
    display: flex;
    gap: 0.25rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .nav-links a {
    color: var(--color-text-secondary);
    text-decoration: none;
    font-weight: 500;
    font-size: 0.875rem;
    padding: 0.5rem 0.875rem;
    border-radius: var(--radius-sm);
    transition: color 0.2s, background 0.2s;
    letter-spacing: 0.01em;
  }

  .nav-links a:hover {
    color: var(--color-text);
    background: var(--color-surface);
  }

  .nav-links a.active {
    color: var(--color-accent);
    background: var(--color-accent-muted);
  }

  /* Auth section */
  .auth-section {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .user-info-link {
    text-decoration: none;
  }

  .user-avatar {
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--color-accent), var(--color-rose));
    color: var(--color-bg);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.8rem;
    font-family: var(--font-display);
    transition: transform 0.2s, box-shadow 0.2s;
  }

  .user-avatar:hover {
    transform: scale(1.08);
    box-shadow: 0 0 16px rgba(212, 175, 55, 0.2);
  }

  .btn-login,
  .btn-logout,
  .btn-register {
    padding: 0.5rem 1rem;
    border-radius: var(--radius-sm);
    font-weight: 500;
    text-decoration: none;
    font-size: 0.8rem;
    transition: all 0.2s;
    cursor: pointer;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    font-family: var(--font-body);
  }

  .btn-login {
    color: var(--color-text-secondary);
    background: transparent;
    border: 1px solid var(--color-border);
  }

  .btn-login:hover {
    border-color: var(--color-text-muted);
    color: var(--color-text);
  }

  .btn-register {
    color: var(--color-bg);
    background: var(--color-accent);
    border: 1px solid var(--color-accent);
  }

  .btn-register:hover {
    background: var(--color-accent-hover);
    border-color: var(--color-accent-hover);
  }

  .btn-logout {
    color: var(--color-text-muted);
    background: transparent;
    border: 1px solid var(--color-border);
  }

  .btn-logout:hover {
    border-color: var(--color-error);
    color: var(--color-error);
  }

  /* ---- Main ---- */
  .main {
    flex: 1;
  }

  /* ---- Footer ---- */
  .footer {
    border-top: 1px solid var(--color-border-subtle);
    padding: 2.5rem 2rem;
  }

  .footer-inner {
    max-width: 1200px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .footer-brand {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .footer-logo {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 1rem;
    color: var(--color-text-heading);
    letter-spacing: -0.01em;
  }

  .footer-divider {
    width: 1px;
    height: 14px;
    background: var(--color-border);
  }

  .footer-location {
    font-size: 0.8rem;
    color: var(--color-text-muted);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .footer-credit {
    font-size: 0.75rem;
    color: var(--color-text-muted);
    margin: 0;
  }

  /* ---- Error ---- */
  .error-container {
    text-align: center;
    padding: 6rem 2rem;
    animation: fadeInUp 0.5s ease both;
  }

  .error-icon {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: var(--color-error-muted);
    color: var(--color-error);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.5rem;
    font-weight: 700;
    margin: 0 auto 1.5rem;
    font-family: var(--font-display);
  }

  .error-container h1 {
    color: var(--color-text-heading);
    margin-bottom: 0.75rem;
    font-size: 1.5rem;
  }

  .error-container p {
    color: var(--color-text-secondary);
    margin-bottom: 0.5rem;
  }

  .hint {
    font-size: 0.8rem;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
  }

  /* ---- Responsive ---- */
  @media (max-width: 768px) {
    .nav {
      flex-wrap: wrap;
      padding: 0.875rem 1.25rem;
    }

    .mobile-toggle {
      display: flex;
    }

    .nav-links {
      display: none;
      flex-direction: column;
      width: 100%;
      gap: 0.25rem;
      padding-top: 0.75rem;
    }

    .nav-links.open {
      display: flex;
    }

    .nav-links a {
      padding: 0.75rem 1rem;
    }

    .auth-section {
      flex-wrap: wrap;
      justify-content: center;
    }

    .footer-inner {
      flex-direction: column;
      gap: 0.75rem;
      text-align: center;
    }
  }
</style>
