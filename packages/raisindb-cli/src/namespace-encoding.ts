/**
 * Filesystem-safe encoding for namespaced names.
 *
 * Names containing colons like `raisin:access_control` are invalid on Windows.
 * Following the Apache Jackrabbit FileVault convention, we encode the namespace
 * prefix by wrapping it in underscores and removing the colon.
 *
 * Encoding is applied ONLY at filesystem/ZIP boundaries.
 */

/** Encode a namespaced name for filesystem use. */
export function encodeNamespace(name: string): string {
  const pos = name.indexOf(':');
  if (pos === -1) return name;
  const prefix = name.substring(0, pos);
  const rest = name.substring(pos + 1);
  return `_${prefix}_${rest}`;
}

/** Decode a filesystem-encoded name back to its logical namespaced form. */
export function decodeNamespace(name: string): string {
  if (!name.startsWith('_') || name.startsWith('__')) return name;
  const pos = name.indexOf('_', 1);
  if (pos === -1) return name;
  const prefix = name.substring(1, pos);
  const rest = name.substring(pos + 1);
  return `${prefix}:${rest}`;
}
