<script lang="ts">
  import { page } from '$app/stores';
  import { goto, invalidateAll } from '$app/navigation';
  import { onMount } from 'svelte';
  import { type NavItem } from '$lib/raisin';
  import { setNavigation, setCurrentPath } from '$lib/stores/navigation';
  import { auth, user } from '$lib/stores/auth';
  import { connection, connected } from '$lib/stores/connection';
  import { chatStore } from '$lib/stores/chat';
  import { messagingStore } from '$lib/stores/messaging-store';
  import { notificationStore } from '$lib/stores/notifications';
  import { presenceStore } from '$lib/stores/presence';
  import ChatWidget from '$lib/components/ChatWidget.svelte';
  import AIChatWidget from '$lib/components/AIChatWidget.svelte';
  import NotificationBell from '$lib/components/NotificationBell.svelte';
  import VoiceActivation from '$lib/components/VoiceActivation.svelte';
  import ToastContainer from '$lib/components/ToastContainer.svelte';
  import type { LayoutData } from './$types';
  import '../app.css';

  interface Props {
    data: LayoutData;
    children: import('svelte').Snippet;
  }

  let { data, children }: Props = $props();

  // Unified store initialization and cleanup
  $effect(() => {
    const userObj = $user;
    if (userObj?.home) {
      // Initialize messaging store first (single source of truth for chat data)
      messagingStore.init();
      // Chat store now only manages UI state (open/minimized popups)
      chatStore.init();
      notificationStore.init();
      presenceStore.init();
    } else {
      messagingStore.reset();
      chatStore.reset();
      notificationStore.reset();
      presenceStore.reset();
    }
  });

  // Initialize connection tracking on mount
  onMount(() => {
    connection.init();
    return () => connection.cleanup();
  });

  // Sync load data with stores on initial load
  $effect(() => {
    if (data.navigationItems) {
      setNavigation(data.navigationItems as NavItem[]);
    }
    if (data.user !== undefined) {
      auth.setUser(data.user);
    }
  });

  // Update current path when page changes
  $effect(() => {
    setCurrentPath($page.url.pathname);
  });

  async function handleLogout() {
    await auth.logout();
    await invalidateAll();
    goto('/');
  }
</script>

<div class="app">
  <header class="header">
    <nav class="nav">
      <a href="/" class="logo">
        <span class="connection-dot" class:connected={$connected} title={$connected ? 'Connected' : 'Disconnected'}></span>
        Launchpad
      </a>

      {#if !data.error}
        <ul class="nav-links">
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
        </ul>
      {/if}

      <div class="auth-section">
        {#if $user}
          <VoiceActivation />
          <NotificationBell />
          <a href="/profile" class="user-info-link">
            <div class="user-info">
              <span class="user-name">{$user.displayName || $user.email}</span>
            </div>
          </a>
          <button class="btn-logout" onclick={handleLogout}>Logout</button>
        {:else}
          <a href="/auth/login" class="btn-login">Login</a>
          <a href="/auth/register" class="btn-register">Register</a>
        {/if}
      </div>
    </nav>
  </header>

  <main class="main">
    {#if data.error}
      <div class="error-container">
        <h1>Connection Error</h1>
        <p>{data.error}</p>
        <p class="hint">Make sure RaisinDB is running on localhost:8081</p>
      </div>
    {:else}
      {@render children()}
    {/if}
  </main>

  <footer class="footer">
    <p>Launchpad Demo - Powered by RaisinDB</p>
  </footer>
</div>

<!-- Chat Widget - floating at bottom right -->
<ChatWidget />

<!-- AI Chat Widget - floating next to Chat Widget -->
<AIChatWidget />

<!-- Toast notifications -->
<ToastContainer />

<style>
  .app {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
  }

  .header {
    background: white;
    border-bottom: 1px solid #e5e7eb;
    position: sticky;
    top: 0;
    z-index: 100;
  }

  .nav {
    max-width: 1200px;
    margin: 0 auto;
    padding: 1rem 2rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .logo {
    font-size: 1.5rem;
    font-weight: 700;
    color: #6366f1;
    text-decoration: none;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .connection-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: #dc2626;
    transition: background-color 0.3s;
  }

  .connection-dot.connected {
    background-color: #16a34a;
  }

  .nav-links {
    display: flex;
    gap: 2rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .nav-links a {
    color: #4b5563;
    text-decoration: none;
    font-weight: 500;
    transition: color 0.2s;
  }

  .nav-links a:hover,
  .nav-links a.active {
    color: #6366f1;
  }

  .auth-section {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .user-info-link {
    text-decoration: none;
    padding: 0.5rem 0.75rem;
    border-radius: 8px;
    transition: background 0.2s;
  }

  .user-info-link:hover {
    background: #f3f4f6;
  }

  .user-info {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0.125rem;
  }

  .user-name {
    color: #4b5563;
    font-weight: 500;
  }

  .btn-login,
  .btn-logout,
  .btn-register {
    padding: 0.5rem 1rem;
    border-radius: 6px;
    font-weight: 500;
    text-decoration: none;
    font-size: 0.875rem;
    transition: background 0.2s, color 0.2s;
    cursor: pointer;
  }

  .btn-login {
    color: #6366f1;
    background: transparent;
    border: 1px solid #6366f1;
  }

  .btn-login:hover {
    background: #f5f3ff;
  }

  .btn-register {
    color: white;
    background: #6366f1;
    border: 1px solid #6366f1;
  }

  .btn-register:hover {
    background: #4f46e5;
  }

  .btn-logout {
    color: #6b7280;
    background: transparent;
    border: 1px solid #d1d5db;
  }

  .btn-logout:hover {
    background: #f3f4f6;
    color: #374151;
  }

  .main {
    flex: 1;
  }

  .footer {
    background: #1f2937;
    color: #9ca3af;
    text-align: center;
    padding: 2rem;
    font-size: 0.875rem;
  }

  .error-container {
    text-align: center;
    padding: 4rem 2rem;
  }

  .error-container h1 {
    color: #991b1b;
    margin-bottom: 1rem;
  }

  .error-container p {
    color: #6b7280;
    margin-bottom: 0.5rem;
  }

  .hint {
    font-size: 0.875rem;
    color: #9ca3af;
  }

  @media (max-width: 768px) {
    .nav {
      flex-direction: column;
      gap: 1rem;
    }

    .nav-links {
      gap: 1rem;
    }

    .auth-section {
      flex-wrap: wrap;
      justify-content: center;
    }
  }
</style>