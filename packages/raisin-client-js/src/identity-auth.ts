/**
 * IdentityAuthApi — passwordless and password sign-in for end users.
 *
 * This is the *identity* half of authentication (the people using an
 * application), distinct from {@link AuthManager}, which is the admin/JWT
 * credential holder the transports authenticate with. The two meet at the
 * token: everything here returns an {@link IdentityAuthResponse}, and
 * `client.setIdentityTokens(...)` hands it to the AuthManager so subsequent
 * SQL, node and function calls run as that person.
 *
 * Every endpoint is repository-scoped (`/auth/{repo}/…`). A repository is not
 * optional: identities, their home nodes and their roles are per repository,
 * so a call that omitted it would have to guess which one, and guessing wrong
 * signs somebody into the wrong data.
 */

/** Transport hook: performs one JSON HTTP call and returns the parsed body. */
export type IdentityAuthTransport = <T>(options: {
  method: string;
  path: string;
  body?: unknown;
  /** Send no Authorization header — the point of a sign-in call. */
  skipAuth?: boolean;
}) => Promise<T>;

/** Identity returned alongside a token pair. */
export interface IdentityInfo {
  id: string;
  email: string;
  display_name: string | null;
  avatar_url: string | null;
  email_verified: boolean;
  linked_providers: string[];
  /** The person's home node path in this repository. */
  home: string | null;
}

/** A successful sign-in: tokens plus who they belong to. */
export interface IdentityAuthResult {
  access_token: string;
  refresh_token: string;
  token_type: string;
  /** Unix seconds. */
  expires_at: number;
  identity: IdentityInfo;
}

/**
 * Acknowledgement that a magic link was *sent*.
 *
 * Deliberately carries no token and no unmasked address: this response is
 * shown to whoever typed the email, who is not yet proven to own it.
 */
export interface MagicLinkSent {
  message: string;
  /** e.g. `u***@example.com` — enough to spot a typo, not to harvest. */
  masked_email: string;
  expires_in_minutes: number;
}

export interface MagicLinkOptions {
  /**
   * Where RaisinDB sends the browser after it verifies the token.
   *
   * The server checks this against the tenant's redirect allowlist and
   * refuses anything outside it, so an attacker cannot aim a victim's
   * one-time token at their own host. Omit it to use the configured default.
   */
  redirectUrl?: string;
}

/** Which sign-in methods this repository actually offers. */
export interface AuthProviders {
  providers: Array<{
    id: string;
    display_name: string;
    icon: string;
    auth_url: string;
  }>;
  local_enabled: boolean;
  magic_link_enabled: boolean;
}

/** Who the caller currently is, as this repository sees them. */
export interface IdentityMe {
  id: string;
  email: string | null;
  roles: string[];
  anonymous: boolean;
  home: string | null;
}

/**
 * Identity authentication scoped to one repository.
 *
 * Obtain it from `client.auth(repository)` rather than constructing it.
 */
export class IdentityAuthApi {
  constructor(
    private readonly repository: string,
    private readonly transport: IdentityAuthTransport,
  ) {}

  private get base(): string {
    return `/auth/${encodeURIComponent(this.repository)}`;
  }

  /**
   * Ask RaisinDB to email a sign-in link.
   *
   * The reply is an acknowledgement, never a token — the token only exists in
   * the email. It also resolves for an address that has no account, because a
   * response that differed would turn this endpoint into an account-existence
   * oracle.
   */
  async sendMagicLink(email: string, options?: MagicLinkOptions): Promise<MagicLinkSent> {
    return this.transport<MagicLinkSent>({
      method: 'POST',
      path: `${this.base}/magic-link`,
      body: { email, redirect_url: options?.redirectUrl },
      skipAuth: true,
    });
  }

  /**
   * Exchange a magic-link token for a session.
   *
   * Most applications never call this: following the emailed link hits
   * RaisinDB directly, which verifies and 302s to the redirect URL with the
   * tokens in the URL **fragment** (so they stay out of server logs and out
   * of the Referer header) — see {@link readTokensFromFragment}. Call this
   * only when handling the token yourself, and note it is single-use.
   */
  async verifyMagicLink(token: string): Promise<IdentityAuthResult> {
    return this.transport<IdentityAuthResult>({
      method: 'GET',
      path: `${this.base}/magic-link/verify?token=${encodeURIComponent(token)}`,
      skipAuth: true,
    });
  }

  /** Sign in with email and password, where the repository enables it. */
  async login(
    email: string,
    password: string,
    options?: { rememberMe?: boolean },
  ): Promise<IdentityAuthResult> {
    return this.transport<IdentityAuthResult>({
      method: 'POST',
      path: `${this.base}/login`,
      body: { email, password, remember_me: options?.rememberMe ?? false },
      skipAuth: true,
    });
  }

  /** Create an account with email and password. */
  async register(
    email: string,
    password: string,
    options?: { displayName?: string },
  ): Promise<IdentityAuthResult> {
    return this.transport<IdentityAuthResult>({
      method: 'POST',
      path: `${this.base}/register`,
      body: { email, password, display_name: options?.displayName },
      skipAuth: true,
    });
  }

  /** Trade a refresh token for a fresh pair. */
  async refresh(refreshToken: string): Promise<IdentityAuthResult> {
    return this.transport<IdentityAuthResult>({
      method: 'POST',
      path: `${this.base}/refresh`,
      body: { refresh_token: refreshToken },
      skipAuth: true,
    });
  }

  /**
   * Which sign-in methods to render.
   *
   * Ask rather than hard-code: magic link and password are per-repository
   * configuration, and a button for a disabled method is a dead end.
   */
  async providers(): Promise<AuthProviders> {
    return this.transport<AuthProviders>({
      method: 'GET',
      path: `${this.base}/providers`,
      skipAuth: true,
    });
  }

  /** Who the current token belongs to in this repository. */
  async me(): Promise<IdentityMe> {
    return this.transport<IdentityMe>({ method: 'GET', path: `${this.base}/me` });
  }
}

/**
 * Read the tokens a magic-link verify redirect left in the URL fragment.
 *
 * Pass `window.location.hash` (or the whole URL) on the callback route. The
 * fragment never reaches a server, which is why the tokens travel there.
 *
 * Returns `null` when the fragment carries no tokens, so a plain visit to the
 * callback route is distinguishable from an arrival from a link.
 */
export function readTokensFromFragment(
  hashOrUrl: string,
): { accessToken: string; refreshToken: string | null } | null {
  const hash = hashOrUrl.includes('#') ? hashOrUrl.slice(hashOrUrl.indexOf('#') + 1) : hashOrUrl;
  if (!hash) return null;
  const params = new URLSearchParams(hash);
  const accessToken = params.get('access_token');
  if (!accessToken) return null;
  return { accessToken, refreshToken: params.get('refresh_token') };
}
