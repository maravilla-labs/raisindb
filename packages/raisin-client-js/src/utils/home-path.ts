/**
 * User home path normalization.
 *
 * Some server responses (JWT claims, RAISIN_CURRENT_USER(), identity
 * endpoints) return the user home as a workspace-prefixed path like
 * `/raisin:access_control/users/internal/alice`, while others return the
 * workspace-relative form `/users/internal/alice`. SDK code that builds
 * child paths (inbox, chats, outbox) expects the workspace-relative form.
 */

/** Workspace prefix that may leak into user home paths */
const ACCESS_CONTROL_PREFIX = '/raisin:access_control';

/**
 * Normalize a user home path to its workspace-relative form.
 *
 * Strips a leading `/raisin:access_control` workspace prefix and trailing
 * slashes, so `/raisin:access_control/users/internal/alice/` becomes
 * `/users/internal/alice`. Paths without the prefix are returned unchanged
 * (modulo trailing-slash trimming). Returns `null` for empty input.
 *
 * @example
 * ```typescript
 * normalizeHomePath('/raisin:access_control/users/internal/alice');
 * // => '/users/internal/alice'
 * normalizeHomePath('/users/internal/alice');
 * // => '/users/internal/alice'
 * ```
 */
export function normalizeHomePath(home: string | null | undefined): string | null {
  if (home === null || home === undefined) return null;

  let path = home.trim();
  if (path.length === 0) return null;

  if (path === ACCESS_CONTROL_PREFIX) return '/';
  if (path.startsWith(`${ACCESS_CONTROL_PREFIX}/`)) {
    path = path.slice(ACCESS_CONTROL_PREFIX.length);
  }

  // Trim trailing slashes (but keep root "/")
  while (path.length > 1 && path.endsWith('/')) {
    path = path.slice(0, -1);
  }

  return path;
}
