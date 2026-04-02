/**
 * Filesystem-safe encoding for namespaced names.
 *
 * Names containing colons like `raisin:access_control` are invalid on Windows.
 * We encode the namespace prefix with a leading underscore and replace the colon
 * with a double underscore (`__`):
 *
 *   raisin:access_control → _raisin__access_control
 *
 * The double underscore makes the encoding unambiguous — names with single
 * underscores (like `my_app:thing`) round-trip correctly.
 *
 * Encoding is applied ONLY at filesystem/ZIP boundaries.
 */

/** Encode a namespaced name for filesystem use. */
export function encodeNamespace(name: string): string {
  const pos = name.indexOf(':');
  if (pos === -1) return name;
  const prefix = name.substring(0, pos);
  const rest = name.substring(pos + 1);
  return `_${prefix}__${rest}`;
}

/** Decode a filesystem-encoded name back to its logical namespaced form. */
export function decodeNamespace(name: string): string {
  if (!name.startsWith('_') || name.startsWith('__')) return name;
  const pos = name.indexOf('__', 1);
  if (pos === -1) return name;
  const prefix = name.substring(1, pos);
  const rest = name.substring(pos + 2);
  return `${prefix}:${rest}`;
}
