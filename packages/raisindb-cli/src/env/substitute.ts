/**
 * `{env:NAME}` substitution for package YAML.
 *
 * A package source directory should be environment-agnostic: the same YAML
 * should build a `.rap` for dev, staging and prod. Instead of hardcoding
 * `base_url: http://localhost:5173`, an author writes
 * `base_url: "{env:PREVIEW_SERVER:-http://localhost:5173}"` and the CLI resolves
 * it when the package is built, validated or pushed.
 *
 * This module is the ONE implementation of the token grammar. Every call site
 * (pack, validate, sync push, .raisin-sync.yaml) goes through it — a second
 * copy would drift and leave one surface shipping raw tokens while another
 * ships resolved values.
 *
 * Deliberately distinct from the flow engine's `{{...}}` / `${...}` templates
 * (see src/flow/template-check.ts), which must pass through untouched.
 */

/** Resolved environment values plus the files they came from (for error text). */
export interface EnvContext {
  values: Record<string, string>;
  /** Human-readable list of contributing sources, lowest precedence first. */
  sources: string[];
}

/** A `{env:NAME}` occurrence that had neither a value nor an inline default. */
export interface UnresolvedToken {
  name: string;
  /** 1-based line number within the file. */
  line: number;
  /** 1-based column of the opening brace. */
  column: number;
  /** The literal token text, e.g. `{env:PREVIEW_SERVER}`. */
  raw: string;
}

export interface SubstitutionResult {
  text: string;
  unresolved: UnresolvedToken[];
}

/**
 * `{env:NAME}` or `{env:NAME:-default}`, optionally escaped with a backslash.
 *
 * The default may contain anything except `}`, which keeps the grammar
 * unambiguous without needing a real parser. A URL default is fine;
 * a default containing a closing brace is not (use a plain value in that case).
 */
const TOKEN_RE = /(\\?)\{env:([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}/g;

/** Cheap pre-check so callers can skip files that need no work at all. */
export function hasEnvTokens(content: string): boolean {
  return content.includes('{env:');
}

/** An empty context — used where substitution is structurally required but no
 *  environment has been loaded (inline defaults still apply). */
export function emptyEnvContext(): EnvContext {
  return { values: {}, sources: [] };
}

/**
 * Replace every `{env:...}` token in `content`.
 *
 * Never throws: unresolved tokens are returned so the caller can decide between
 * aborting the whole build (pack) and failing one file (sync).
 */
export function substituteEnvTokens(content: string, env: EnvContext): SubstitutionResult {
  const unresolved: UnresolvedToken[] = [];

  if (!hasEnvTokens(content)) {
    return { text: content, unresolved };
  }

  // Offsets of each line start, so a match index becomes line/column.
  const lineStarts: number[] = [0];
  for (let i = 0; i < content.length; i++) {
    if (content[i] === '\n') lineStarts.push(i + 1);
  }

  const positionOf = (index: number): { line: number; column: number } => {
    // Binary search for the last line start <= index.
    let lo = 0;
    let hi = lineStarts.length - 1;
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      if (lineStarts[mid] <= index) lo = mid;
      else hi = mid - 1;
    }
    return { line: lo + 1, column: index - lineStarts[lo] + 1 };
  };

  TOKEN_RE.lastIndex = 0;
  const text = content.replace(
    TOKEN_RE,
    (match, escape: string, name: string, fallback: string | undefined, offset: number) => {
      // `\{env:X}` is an escape hatch so a package can document the syntax in
      // its own comments without the CLI eating it.
      if (escape) {
        return match.slice(1);
      }

      const value = env.values[name];
      if (value !== undefined) {
        return value;
      }
      if (fallback !== undefined) {
        return fallback;
      }

      const { line, column } = positionOf(offset);
      unresolved.push({ name, line, column, raw: match });
      // Leave the token in place; the caller aborts rather than shipping this.
      return match;
    }
  );

  return { text, unresolved };
}

/**
 * Format unresolved tokens as a multi-line error message.
 *
 * `entries` is keyed by the path to show for each file (relative paths read
 * better than absolute ones in CLI output).
 */
export function formatUnresolvedError(
  entries: Array<{ path: string; unresolved: UnresolvedToken[] }>,
  env: EnvContext
): string {
  const lines: string[] = [];
  const names = new Set<string>();

  lines.push('Unresolved {env:...} tokens:');
  for (const entry of entries) {
    for (const token of entry.unresolved) {
      names.add(token.name);
      lines.push(`  ${entry.path}:${token.line}:${token.column}  ${token.raw}`);
    }
  }

  lines.push('');
  lines.push(
    `Set ${[...names].map((n) => `${n}`).join(', ')} in the environment or a .env file, ` +
      'or give the token an inline default: {env:NAME:-fallback}'
  );
  if (env.sources.length > 0) {
    lines.push(`Env sources consulted: ${env.sources.join(', ')}`);
  } else {
    lines.push('Env sources consulted: process environment only (no .env file found)');
  }

  return lines.join('\n');
}
