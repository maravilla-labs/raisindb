<script lang="ts">
  import { goto, invalidateAll } from '$app/navigation';
  import { auth } from '$lib/stores/auth';

  let email = $state('');
  let password = $state('');
  let confirmPassword = $state('');
  let displayName = $state('');
  let error = $state<string | null>(null);
  let submitting = $state(false);

  async function handleSubmit(e: Event) {
    e.preventDefault();
    error = null;

    if (password !== confirmPassword) {
      error = 'Passwords do not match';
      return;
    }

    if (password.length < 8) {
      error = 'Password must be at least 8 characters';
      return;
    }

    submitting = true;

    const result = await auth.register(email, password, displayName || undefined);

    if (result.success) {
      await invalidateAll();
      goto('/');
    } else {
      error = result.error.message;
      submitting = false;
    }
  }
</script>

<svelte:head>
  <title>Join - Nachtkultur</title>
</svelte:head>

<div class="auth-container">
  <div class="auth-wrapper">
    <div class="auth-glow"></div>

    <div class="auth-card">
      <div class="auth-header">
        <h1>Join Nachtkultur</h1>
        <p class="subtitle">Create your account</p>
      </div>

      {#if error}
        <div class="error-message">{error}</div>
      {/if}

      <form onsubmit={handleSubmit}>
        <div class="form-group">
          <label for="displayName">Display Name</label>
          <input
            type="text"
            id="displayName"
            bind:value={displayName}
            placeholder="Your name"
            disabled={submitting}
          />
        </div>

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
            placeholder="At least 8 characters"
            required
            minlength="8"
            disabled={submitting}
          />
        </div>

        <div class="form-group">
          <label for="confirmPassword">Confirm Password</label>
          <input
            type="password"
            id="confirmPassword"
            bind:value={confirmPassword}
            placeholder="Confirm your password"
            required
            disabled={submitting}
          />
        </div>

        <button type="submit" class="btn-primary" disabled={submitting}>
          {submitting ? 'Creating account...' : 'Create Account'}
        </button>
      </form>

      <p class="auth-link">
        Already a member? <a href="/auth/login">Sign in</a>
      </p>
    </div>
  </div>
</div>

<style>
  .auth-container {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 70vh;
    padding: 2rem;
  }

  .auth-wrapper {
    position: relative;
    width: 100%;
    max-width: 400px;
  }

  .auth-glow {
    position: absolute;
    top: -60px;
    left: 50%;
    transform: translateX(-50%);
    width: 300px;
    height: 200px;
    background: radial-gradient(ellipse, rgba(212, 175, 55, 0.06) 0%, transparent 70%);
    pointer-events: none;
  }

  .auth-card {
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    padding: 2.5rem;
    position: relative;
    animation: fadeInUp 0.5s ease both;
  }

  .auth-header {
    margin-bottom: 2rem;
  }

  h1 {
    margin: 0 0 0.375rem 0;
    font-family: var(--font-display);
    font-size: 1.5rem;
    font-weight: 600;
    color: var(--color-text-heading);
    letter-spacing: -0.02em;
  }

  .subtitle {
    color: var(--color-text-muted);
    margin: 0;
    font-size: 0.9rem;
  }

  .error-message {
    background: var(--color-error-muted);
    border: 1px solid rgba(239, 68, 68, 0.2);
    color: var(--color-error);
    padding: 0.75rem 1rem;
    border-radius: var(--radius-md);
    margin-bottom: 1.25rem;
    font-size: 0.85rem;
  }

  .form-group {
    margin-bottom: 1.25rem;
  }

  label {
    display: block;
    font-weight: 500;
    color: var(--color-text-secondary);
    margin-bottom: 0.5rem;
    font-size: 0.8rem;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  input {
    width: 100%;
    padding: 0.75rem 1rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    font-size: 0.95rem;
    color: var(--color-text);
    font-family: var(--font-body);
    transition: border-color 0.2s, box-shadow 0.2s;
  }

  input::placeholder {
    color: var(--color-text-muted);
  }

  input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px var(--color-accent-glow);
  }

  input:disabled {
    opacity: 0.5;
  }

  .btn-primary {
    width: 100%;
    padding: 0.8rem 1rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-md);
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
    margin-top: 0.5rem;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    font-family: var(--font-body);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
    box-shadow: 0 4px 20px rgba(212, 175, 55, 0.2);
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .auth-link {
    text-align: center;
    margin-top: 1.75rem;
    color: var(--color-text-muted);
    font-size: 0.85rem;
  }

  .auth-link a {
    color: var(--color-accent);
    text-decoration: none;
    font-weight: 500;
  }

  .auth-link a:hover {
    color: var(--color-accent-hover);
  }
</style>
