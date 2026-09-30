import http from 'http';
import open from 'open';
import {
  getEffectiveServer,
  loadConfig,
  normalizeServerUrl,
  sameServer,
  saveConfig,
} from './config.js';

const CALLBACK_PORT = 9999;
const CALLBACK_PATH = '/auth/callback';
const LOGIN_TIMEOUT_MS = 2 * 60 * 1000; // 2 minutes

// Store active login state for cancellation
let activeLoginServer: http.Server | null = null;
let activeLoginTimeout: NodeJS.Timeout | null = null;

/**
 * Cancel any active login attempt
 */
export function cancelLogin(): void {
  if (activeLoginTimeout) {
    clearTimeout(activeLoginTimeout);
    activeLoginTimeout = null;
  }
  if (activeLoginServer) {
    activeLoginServer.close();
    activeLoginServer = null;
  }
}

/**
 * Opens browser for authentication and waits for callback token
 */
export async function login(serverUrl: string): Promise<string> {
  // Cancel any existing login attempt
  cancelLogin();

  return new Promise((resolve, reject) => {
    activeLoginTimeout = setTimeout(() => {
      if (activeLoginServer) {
        activeLoginServer.close();
        activeLoginServer = null;
      }
      activeLoginTimeout = null;
      reject(new Error('Authentication timeout - no response received'));
    }, LOGIN_TIMEOUT_MS);

    // Create temporary HTTP server to receive callback
    activeLoginServer = http.createServer((req, res) => {
      if (req.url?.startsWith(CALLBACK_PATH)) {
        const url = new URL(req.url, `http://localhost:${CALLBACK_PORT}`);
        const token = url.searchParams.get('token');

        if (token) {
          // Send simple OK response (server shows the success page now)
          res.writeHead(200, { 'Content-Type': 'text/plain' });
          res.end('OK');

          // Save token to config
          const config = loadConfig();
          config.token = token;
          config.server = serverUrl;
          saveConfig(config);

          // Clean up
          if (activeLoginTimeout) {
            clearTimeout(activeLoginTimeout);
            activeLoginTimeout = null;
          }
          if (activeLoginServer) {
            activeLoginServer.close();
            activeLoginServer = null;
          }
          resolve(token);
        } else {
          res.writeHead(400, { 'Content-Type': 'text/plain' });
          res.end('Missing token parameter');
          if (activeLoginTimeout) {
            clearTimeout(activeLoginTimeout);
            activeLoginTimeout = null;
          }
          if (activeLoginServer) {
            activeLoginServer.close();
            activeLoginServer = null;
          }
          reject(new Error('Authentication failed - no token received'));
        }
      } else {
        res.writeHead(404, { 'Content-Type': 'text/plain' });
        res.end('Not found');
      }
    });

    activeLoginServer.listen(CALLBACK_PORT, () => {
      // Open browser to server's login page
      // Only include port parameter if not using default port
      let loginUrl = `${serverUrl}/auth/cli`;
      if (CALLBACK_PORT !== 9999) {
        loginUrl += `?port=${CALLBACK_PORT}`;
      }

      open(loginUrl).catch((error) => {
        if (activeLoginTimeout) {
          clearTimeout(activeLoginTimeout);
          activeLoginTimeout = null;
        }
        if (activeLoginServer) {
          activeLoginServer.close();
          activeLoginServer = null;
        }
        reject(new Error(`Failed to open browser: ${error.message}`));
      });
    });

    activeLoginServer.on('error', (error) => {
      if (activeLoginTimeout) {
        clearTimeout(activeLoginTimeout);
        activeLoginTimeout = null;
      }
      activeLoginServer = null;
      reject(new Error(`Failed to start callback server: ${error.message}`));
    });
  });
}

/**
 * Clears the stored authentication token
 */
export function logout(): void {
  const config = loadConfig();
  config.token = null;
  saveConfig(config);
}

/**
 * Checks if user is authenticated
 */
export function isAuthenticated(): boolean {
  const config = loadConfig();
  return !!config.token;
}

/** Targets already warned about, so a command's many requests warn once. */
const warnedTargets = new Set<string>();

/** Forget which targets were warned about (tests). */
export function resetTokenWarnings(): void {
  warnedTargets.clear();
}

/**
 * Gets the authentication token for a request to `targetServer` (default: the
 * server this command talks to, `getEffectiveServer()`).
 *
 * Resolution order:
 *   1. RAISINDB_TOKEN environment variable — an explicit choice, sent as is
 *   2. the .raisinrc token, but ONLY when `targetServer` is the server that
 *      token was issued for (the one stored next to it by `raisindb login`)
 *
 * A saved login for another server yields `null` and a one-time hint on
 * stderr — sending server A's token to server B would hand B a credential
 * for A.
 */
export function getToken(targetServer?: string | null): string | null {
  const envToken = process.env.RAISINDB_TOKEN;
  if (envToken && envToken.trim() !== '') {
    return envToken.trim();
  }
  const config = loadConfig();
  if (!config.token) {
    return null;
  }
  const target = targetServer || getEffectiveServer();
  if (sameServer(target, config.server)) {
    return config.token;
  }
  const key = normalizeServerUrl(target);
  if (!warnedTargets.has(key)) {
    warnedTargets.add(key);
    console.error(
      `Note: the saved login is for ${config.server ?? 'an unknown server'}, not ${target}; ` +
        'sending no credentials.\n' +
        `  Log in to it with: raisindb login -s ${target}   (or set RAISINDB_TOKEN)`
    );
  }
  return null;
}

export interface PasswordLoginResult {
  token: string;
  /** Unix timestamp (seconds) when the token expires, if reported by the server */
  expiresAt?: number;
  username?: string;
}

/**
 * Non-interactive login with username/password (system user auth).
 *
 * POSTs to {server}/api/raisindb/sys/{tenant}/auth and stores the
 * resulting token + server in the CLI config (.raisinrc).
 */
export async function loginWithPassword(
  serverUrl: string,
  username: string,
  password: string,
  tenant: string = 'default'
): Promise<PasswordLoginResult> {
  const url = `${serverUrl.replace(/\/$/, '')}/api/raisindb/sys/${encodeURIComponent(tenant)}/auth`;

  const response = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });

  if (!response.ok) {
    const text = await response.text().catch(() => '');
    throw new Error(`Authentication failed: HTTP ${response.status}${text ? ` - ${text}` : ''}`);
  }

  const data = (await response.json()) as {
    token?: string;
    expires_at?: number;
    username?: string;
  };

  if (!data.token) {
    throw new Error('Authentication response did not contain a token');
  }

  const config = loadConfig();
  config.server = serverUrl;
  config.token = data.token;
  saveConfig(config);

  return { token: data.token, expiresAt: data.expires_at, username: data.username };
}

/**
 * Non-interactive login with an existing token.
 * Stores server + token in the CLI config (.raisinrc).
 */
export function loginWithToken(serverUrl: string, token: string): void {
  const config = loadConfig();
  config.server = serverUrl;
  config.token = token;
  saveConfig(config);
}
