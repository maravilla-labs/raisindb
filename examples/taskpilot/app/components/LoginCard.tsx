/**
 * Login form bound to useAuth().login (client.loginWithEmail under the
 * hood: HTTP login + WebSocket connect + JWT auth in one call).
 * Demo credentials are prefilled.
 */
import { useState, type FormEvent } from 'react';
import type { UseAuthReturn } from '@raisindb/client/react';
import { REPOSITORY } from '../lib/raisin';

export function LoginCard({ auth }: { auth: UseAuthReturn }) {
  const [email, setEmail] = useState('pilot@example.com');
  const [password, setPassword] = useState('Pilot12345!');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await auth.login(email.trim(), password, REPOSITORY);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="login-screen">
      <form className="login-card" onSubmit={submit}>
        <div className="login-brand">
          <span className="logo-mark">✈</span>
          <h1>TaskPilot</h1>
        </div>
        <p className="login-hint">
          RaisinDB plan/task demo — sign in with the prefilled demo account
          (<code>pilot@example.com</code>).
        </p>
        <label>
          Email
          <input
            type="email"
            name="email"
            autoComplete="username"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
          />
        </label>
        <label>
          Password
          <input
            type="password"
            name="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </label>
        {error && (
          <p className="login-error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" disabled={submitting || auth.isLoading}>
          {submitting ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </div>
  );
}
