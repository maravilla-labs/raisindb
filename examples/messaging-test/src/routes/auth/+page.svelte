<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { auth, user, isLoading } from '$lib/stores/auth';
  import { LogIn, UserPlus, Mail, Lock, User } from 'lucide-svelte';

  let mode = $derived($page.url.searchParams.get('mode') === 'register' ? 'register' : 'login');

  let email = $state('');
  let password = $state('');
  let displayName = $state('');
  let error = $state<string | null>(null);
  let success = $state<string | null>(null);

  async function handleLogin() {
    error = null;
    success = null;

    const result = await auth.login(email, password);
    if (result.success) {
      success = 'Login successful!';
      setTimeout(() => goto('/'), 500);
    } else {
      error = result.error.message;
    }
  }

  async function handleRegister() {
    error = null;
    success = null;

    if (!displayName.trim()) {
      error = 'Display name is required';
      return;
    }

    const result = await auth.register(email, password, displayName);
    if (result.success) {
      success = 'Registration successful! You are now logged in.';
      setTimeout(() => goto('/'), 500);
    } else {
      error = result.error.message;
    }
  }

  function switchMode(newMode: 'login' | 'register') {
    error = null;
    success = null;
    goto(`/auth?mode=${newMode}`);
  }
</script>

<div class="auth-page">
  {#if $user}
    <div class="card already-logged-in">
      <h2>Already Logged In</h2>
      <p>You are logged in as <strong>{$user.displayName || $user.id}</strong></p>
      <div class="user-details">
        <div class="detail">
          <span class="label">User ID:</span>
          <span>{$user.id}</span>
        </div>
      </div>
      <div class="actions">
        <a href="/" class="btn btn-primary">Go to Dashboard</a>
        <button class="btn btn-secondary" onclick={() => auth.logout()}>Logout</button>
      </div>
    </div>
  {:else}
    <div class="card auth-card">
      <div class="tabs">
        <button
          class="tab"
          class:active={mode === 'login'}
          onclick={() => switchMode('login')}
        >
          <LogIn size={18} />
          Login
        </button>
        <button
          class="tab"
          class:active={mode === 'register'}
          onclick={() => switchMode('register')}
        >
          <UserPlus size={18} />
          Register
        </button>
      </div>

      {#if error}
        <div class="alert alert-error">{error}</div>
      {/if}

      {#if success}
        <div class="alert alert-success">{success}</div>
      {/if}

      <form onsubmit={(e) => { e.preventDefault(); mode === 'login' ? handleLogin() : handleRegister(); }}>
        {#if mode === 'register'}
          <div class="form-group">
            <label class="form-label" for="displayName">
              <User size={16} />
              Display Name
            </label>
            <input
              type="text"
              id="displayName"
              class="form-input"
              bind:value={displayName}
              placeholder="e.g., Alice"
              required
            />
          </div>
        {/if}

        <div class="form-group">
          <label class="form-label" for="email">
            <Mail size={16} />
            Email
          </label>
          <input
            type="email"
            id="email"
            class="form-input"
            bind:value={email}
            placeholder="e.g., alice@test.com"
            required
          />
        </div>

        <div class="form-group">
          <label class="form-label" for="password">
            <Lock size={16} />
            Password
          </label>
          <input
            type="password"
            id="password"
            class="form-input"
            bind:value={password}
            placeholder="Enter password"
            required
          />
        </div>

        <button type="submit" class="btn btn-primary btn-full" disabled={$isLoading}>
          {#if $isLoading}
            Processing...
          {:else if mode === 'login'}
            <LogIn size={18} />
            Login
          {:else}
            <UserPlus size={18} />
            Create Account
          {/if}
        </button>
      </form>

      <div class="test-accounts">
        <h3>Suggested Test Accounts</h3>
        <div class="account-list">
          <div class="account">
            <strong>Alice</strong>
            <span>alice@test.com / alice123</span>
          </div>
          <div class="account">
            <strong>Bob</strong>
            <span>bob@test.com / bob123</span>
          </div>
          <div class="account">
            <strong>Charlie</strong>
            <span>charlie@test.com / charlie123</span>
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .auth-page {
    max-width: 500px;
    margin: 0 auto;
  }

  .card {
    background: white;
    border-radius: 0.75rem;
    padding: 2rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .auth-card .tabs {
    display: flex;
    gap: 0;
    margin: -2rem -2rem 1.5rem -2rem;
    border-bottom: 1px solid #e5e7eb;
  }

  .auth-card .tab {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 1rem;
    border: none;
    background: #f9fafb;
    color: #6b7280;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .auth-card .tab:first-child {
    border-top-left-radius: 0.75rem;
  }

  .auth-card .tab:last-child {
    border-top-right-radius: 0.75rem;
  }

  .auth-card .tab.active {
    background: white;
    color: #6366f1;
    border-bottom: 2px solid #6366f1;
    margin-bottom: -1px;
  }

  .auth-card .tab:hover:not(.active) {
    background: #f3f4f6;
  }

  .form-group {
    margin-bottom: 1rem;
  }

  .form-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 500;
    margin-bottom: 0.5rem;
    color: #374151;
  }

  .btn-full {
    width: 100%;
    margin-top: 0.5rem;
  }

  .already-logged-in {
    text-align: center;
  }

  .already-logged-in h2 {
    margin: 0 0 1rem 0;
    color: #111827;
  }

  .already-logged-in p {
    margin: 0 0 1.5rem 0;
    color: #6b7280;
  }

  .user-details {
    background: #f9fafb;
    border-radius: 0.5rem;
    padding: 1rem;
    margin-bottom: 1.5rem;
    text-align: left;
  }

  .detail {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }

  .detail:last-child {
    margin-bottom: 0;
  }

  .detail .label {
    font-weight: 500;
    color: #374151;
    min-width: 60px;
  }

  .detail code {
    font-family: monospace;
    font-size: 0.875rem;
    color: #6b7280;
    word-break: break-all;
  }

  .actions {
    display: flex;
    gap: 1rem;
    justify-content: center;
  }

  .test-accounts {
    margin-top: 2rem;
    padding-top: 1.5rem;
    border-top: 1px solid #e5e7eb;
  }

  .test-accounts h3 {
    font-size: 0.875rem;
    font-weight: 600;
    color: #6b7280;
    margin: 0 0 1rem 0;
  }

  .account-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .account {
    display: flex;
    justify-content: space-between;
    padding: 0.5rem 0.75rem;
    background: #f9fafb;
    border-radius: 0.375rem;
    font-size: 0.875rem;
  }

  .account strong {
    color: #374151;
  }

  .account span {
    color: #6b7280;
    font-family: monospace;
  }
</style>
