import { apiCall, readStdin, FetchLike } from './admin-util.js';

/**
 * `raisindb user register` - create an identity user (demo/login user).
 *
 * Primary endpoint (admin-level, creates the identity AND a raisin:User node
 * in the target repo, marks the email verified):
 *   POST /api/raisindb/sys/{tenant}/identity-users
 *        {email, password, display_name?, email_verified, repos:[repo]}
 *
 * Fallback when the token has no admin/console access (401/403):
 *   POST /auth/{repo}/register {email, password, display_name}
 *
 * Both endpoints return 409 EMAIL_EXISTS when the identity already exists
 * (identities are tenant-wide, not per-repo) - handled via --exists-ok.
 */

export interface UserRegisterOptions {
  password?: string;
  passwordStdin?: boolean;
  repo?: string;
  displayName?: string;
  tenant?: string;
  /** Exit successfully if the identity already exists (idempotent CI). */
  existsOk?: boolean;
}

export interface UserRegisterDeps {
  readStdinImpl?: () => Promise<string>;
}

/** Resolve the password from --password or --password-stdin (exactly one). */
export async function resolvePassword(
  options: Pick<UserRegisterOptions, 'password' | 'passwordStdin'>,
  deps: UserRegisterDeps = {}
): Promise<string> {
  if (options.password !== undefined && options.passwordStdin) {
    throw new Error('--password and --password-stdin are mutually exclusive.');
  }
  if (options.password !== undefined) {
    if (!options.password) {
      throw new Error('--password was given an empty value.');
    }
    return options.password;
  }
  if (options.passwordStdin) {
    const reader = deps.readStdinImpl ?? readStdin;
    const value = (await reader()).trim();
    if (!value) {
      throw new Error('No password received on stdin.');
    }
    return value;
  }
  throw new Error('A password is required: pass --password <value> or --password-stdin.');
}

export async function userRegister(
  email: string,
  options: UserRegisterOptions,
  fetchImpl?: FetchLike,
  deps?: UserRegisterDeps
): Promise<void> {
  if (!options.repo) {
    throw new Error('--repo is required (the repository the user should access).');
  }
  const tenant = options.tenant || 'default';
  const password = await resolvePassword(options, deps);

  // Preferred: admin endpoint (verified email + raisin:User node in the repo)
  const adminResult = await apiCall<{ id: string; email: string }>(
    `/api/raisindb/sys/${encodeURIComponent(tenant)}/identity-users`,
    {
      method: 'POST',
      body: {
        email,
        password,
        display_name: options.displayName,
        email_verified: true,
        repos: [options.repo],
      },
      fetchImpl,
    }
  );

  if (adminResult.ok) {
    console.log(`Identity user '${email}' created (repo: ${options.repo}, tenant: ${tenant}).`);
    return;
  }

  if (adminResult.status === 409) {
    if (options.existsOk) {
      console.log(`Identity user '${email}' already exists (ok).`);
      return;
    }
    throw new Error(`Identity user '${email}' already exists (use --exists-ok to ignore).`);
  }

  // Fallback: self-service registration endpoint (no admin claims required)
  if (adminResult.status === 401 || adminResult.status === 403 || adminResult.status === 404) {
    const registerResult = await apiCall<unknown>(
      `/auth/${encodeURIComponent(options.repo)}/register`,
      {
        method: 'POST',
        body: { email, password, display_name: options.displayName },
        fetchImpl,
      }
    );

    if (registerResult.ok) {
      console.log(`Identity user '${email}' registered (repo: ${options.repo}).`);
      return;
    }
    if (registerResult.status === 409) {
      if (options.existsOk) {
        console.log(`Identity user '${email}' already exists (ok).`);
        return;
      }
      throw new Error(`Identity user '${email}' already exists (use --exists-ok to ignore).`);
    }
    throw new Error(`Failed to register user '${email}': ${registerResult.errorMessage}`);
  }

  throw new Error(`Failed to create user '${email}': ${adminResult.errorMessage}`);
}
