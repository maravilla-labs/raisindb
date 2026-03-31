<script lang="ts">
  import { page } from '$app/stores';
  import { goto, invalidateAll } from '$app/navigation';
  import { onMount } from 'svelte';
  import { auth, user } from '$lib/stores/auth';
  import { connection, connected } from '$lib/stores/connection';
  import { messagesStore } from '$lib/stores/messages';
  import { notificationStore } from '$lib/stores/notifications.svelte';
  import NotificationToast from '$lib/components/NotificationToast.svelte';
  import NotificationBell from '$lib/components/NotificationBell.svelte';
  import { Mail, Users, Send, Inbox, Home, LogOut, LogIn, UserPlus, Heart, ClipboardList } from 'lucide-svelte';
  import type { LayoutData } from './$types';
  import '../app.css';

  interface Props {
    data: LayoutData;
    children: import('svelte').Snippet;
  }

  let { data, children }: Props = $props();

  // Initialize stores when user changes
  $effect(() => {
    const userObj = $user;
    if (userObj?.home) {
      messagesStore.init();
      notificationStore.init();
    } else {
      messagesStore.reset();
      notificationStore.reset();
    }
  });

  // Initialize connection tracking
  onMount(() => {
    connection.init();
    return () => {
      connection.cleanup();
      notificationStore.cleanup();
    };
  });

  // Sync load data with stores
  $effect(() => {
    if (data.user !== undefined) {
      auth.setUser(data.user);
    }
  });

  async function handleLogout() {
    await auth.logout();
    await invalidateAll();
    goto('/');
  }

  const navItems = [
    { href: '/', icon: Home, label: 'Dashboard' },
    { href: '/auth', icon: LogIn, label: 'Auth' },
    { href: '/users', icon: Users, label: 'Users' },
    { href: '/inbox', icon: Inbox, label: 'Inbox' },
    { href: '/send', icon: Send, label: 'Send' },
    { href: '/friends', icon: Heart, label: 'Friends' },
    { href: '/tasks', icon: ClipboardList, label: 'Tasks' },
    { href: '/family', icon: UserPlus, label: 'Family' },
  ];
</script>

<div class="app">
  <header class="header">
    <nav class="nav">
      <a href="/" class="logo">
        <span class="connection-dot" class:connected={$connected} title={$connected ? 'Connected' : 'Disconnected'}></span>
        <Mail size={24} />
        Messaging Test
      </a>

      {#if !data.error}
        <ul class="nav-links">
          {#each navItems as item (item.href)}
            {@const IconComponent = item.icon}
            <li>
              <a
                href={item.href}
                class:active={$page.url.pathname === item.href || (item.href !== '/' && $page.url.pathname.startsWith(item.href))}
              >
                <IconComponent size={18} />
                {item.label}
              </a>
            </li>
          {/each}
        </ul>
      {/if}

      <div class="auth-section">
        {#if $user}
          <NotificationBell />
          <div class="user-info">
            <span class="user-name">{$user.displayName || $user.id}</span>
            <span class="user-home">{$user.id}</span>
          </div>
          <button class="btn-logout" onclick={handleLogout}>
            <LogOut size={16} />
            Logout
          </button>
        {:else}
          <a href="/auth" class="btn-login">
            <LogIn size={16} />
            Login
          </a>
          <a href="/auth?mode=register" class="btn-register">
            <UserPlus size={16} />
            Register
          </a>
        {/if}
      </div>
    </nav>
  </header>

  <main class="main">
    {#if data.error}
      <div class="error-container">
        <h1>Connection Error</h1>
        <p>{data.error}</p>
        <p class="hint">Make sure RaisinDB is running on localhost:8081 with the phase1test repository</p>
      </div>
    {:else}
      {@render children()}
    {/if}
  </main>

  <footer class="footer">
    <p>Messaging Test - Using phase1test workspace</p>
  </footer>

  <!-- Toast notifications container -->
  <NotificationToast />
</div>

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
    max-width: 1400px;
    margin: 0 auto;
    padding: 1rem 2rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .logo {
    font-size: 1.25rem;
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
    gap: 0.5rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .nav-links a {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: #4b5563;
    text-decoration: none;
    font-weight: 500;
    padding: 0.5rem 1rem;
    border-radius: 0.5rem;
    transition: all 0.2s;
  }

  .nav-links a:hover {
    background: #f3f4f6;
    color: #6366f1;
  }

  .nav-links a.active {
    background: #eef2ff;
    color: #6366f1;
  }

  .auth-section {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .user-info {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0.125rem;
  }

  .user-name {
    color: #374151;
    font-weight: 500;
    font-size: 0.875rem;
  }

  .user-home {
    color: #9ca3af;
    font-size: 0.75rem;
    font-family: monospace;
  }

  .btn-login,
  .btn-logout,
  .btn-register {
    display: flex;
    align-items: center;
    gap: 0.5rem;
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
    padding: 2rem;
  }

  .footer {
    background: #1f2937;
    color: #9ca3af;
    text-align: center;
    padding: 1rem 2rem;
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
      flex-wrap: wrap;
      justify-content: center;
    }
  }
</style>
