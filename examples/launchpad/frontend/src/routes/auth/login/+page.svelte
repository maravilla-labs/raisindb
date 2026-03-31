<script lang="ts">
  import { goto, invalidateAll } from '$app/navigation';
  import { auth } from '$lib/stores/auth';

  let email = $state('');
  let password = $state('');
  let error = $state<string | null>(null);
  let submitting = $state(false);

  async function handleSubmit(e: Event) {
    e.preventDefault();
    error = null;
    submitting = true;

    const result = await auth.login(email, password);

    if (result.success) {
      // Invalidate all load functions to reload data as authenticated user
      await invalidateAll();
      goto('/');
    } else {
      error = result.error.message;
      submitting = false;
    }
  }
</script>

<svelte:head>
  <title>Login - Launchpad</title>
</svelte:head>

<div class="auth-container">
  <div class="auth-card">
    <h1>Login</h1>
    <p class="subtitle">Welcome back to Launchpad</p>

    {#if error}
      <div class="error-message">{error}</div>
    {/if}

    <form onsubmit={handleSubmit}>
      <div class="form-group">
        <label for="email">Email</label>
        <input
          type="email"
          id="email"
          bind:value={email}
          placeholder="you@example.com"
          required
          disabled={submitting}
        />
      </div>

      <div class="form-group">
        <label for="password">Password</label>
        <input
          type="password"
          id="password"
          bind:value={password}
          placeholder="Your password"
          required
          disabled={submitting}
        />
      </div>

      <button type="submit" class="btn-primary" disabled={submitting}>
        {submitting ? 'Signing in...' : 'Sign In'}
      </button>
    </form>

    <p class="auth-link">
      Don't have an account? <a href="/auth/register">Register</a>
    </p>
  </div>
</div>

<style>
  .auth-container {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 60vh;
    padding: 2rem;
  }

  .auth-card {
    background: white;
    border-radius: 12px;
    box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
    padding: 2.5rem;
    width: 100%;
    max-width: 400px;
  }

  h1 {
    margin: 0 0 0.5rem 0;
    font-size: 1.75rem;
    color: #111827;
  }

  .subtitle {
    color: #6b7280;
    margin: 0 0 1.5rem 0;
  }

  .error-message {
    background: #fef2f2;
    border: 1px solid #fecaca;
    color: #991b1b;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    margin-bottom: 1rem;
    font-size: 0.875rem;
  }

  .form-group {
    margin-bottom: 1rem;
  }

  label {
    display: block;
    font-weight: 500;
    color: #374151;
    margin-bottom: 0.5rem;
    font-size: 0.875rem;
  }

  input {
    width: 100%;
    padding: 0.75rem 1rem;
    border: 1px solid #d1d5db;
    border-radius: 8px;
    font-size: 1rem;
    transition: border-color 0.2s, box-shadow 0.2s;
  }

  input:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  input:disabled {
    background: #f9fafb;
    color: #9ca3af;
  }

  .btn-primary {
    width: 100%;
    padding: 0.75rem 1rem;
    background: #6366f1;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 1rem;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
    margin-top: 0.5rem;
  }

  .btn-primary:hover:not(:disabled) {
    background: #4f46e5;
  }

  .btn-primary:disabled {
    background: #a5b4fc;
    cursor: not-allowed;
  }

  .auth-link {
    text-align: center;
    margin-top: 1.5rem;
    color: #6b7280;
    font-size: 0.875rem;
  }

  .auth-link a {
    color: #6366f1;
    text-decoration: none;
    font-weight: 500;
  }

  .auth-link a:hover {
    text-decoration: underline;
  }
</style>
