import http from 'http';
import open from 'open';
import { loadConfig, saveConfig } from './config.js';

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

/**
 * Gets the current authentication token
 */
export function getToken(): string | null {
  const config = loadConfig();
  return config.token;
}
