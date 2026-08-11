import { apiCall, formatTable, readStdin, FetchLike } from './admin-util.js';
import { getDefaultRepo } from '../config.js';

/**
 * `raisindb secret ...` — the encrypted secret store, gh-secret style: the
 * value is never echoed, never logged, and by default never passed on argv.
 *
 * Endpoints (raisin-transport-http, handlers/secrets):
 *   PUT    /api/secrets/{repo}/{branch}/{name}          { value }
 *   GET    /api/secrets/{repo}/{branch}
 *   GET    /api/secrets/{repo}/{branch}/{name}          (metadata + versions)
 *   DELETE /api/secrets/{repo}/{branch}/{name}
 *   POST   /api/secrets/{repo}/{branch}/rotate/{name}   { value }
 *
 * Note the odd one out: rotate puts the literal segment BEFORE the name. A
 * secret name may contain `/` (the auto-vault convention is
 * `node/{id}/{field.path}`), so the name has to be a wildcard capture — and a
 * wildcard must be the last path segment, which makes `{name}/rotate`
 * unregisterable. Don't "fix" this to read more naturally.
 *
 * Note what is NOT here: there is no `secret get`. The API has no endpoint that
 * returns plaintext, and adding a CLI that printed one would put credentials
 * into terminal scrollback and CI logs. Server-side functions read values
 * through `raisin.secrets.get`, gated by their `secret_policy`.
 */

/** Secrets are branch-scoped: the same name on two branches is two secrets. */
export const DEFAULT_BRANCH = 'main';

/**
 * Metadata as returned by the server. Deliberately has no field that could
 * hold ciphertext or plaintext.
 */
export interface SecretMetadata {
  name: string;
  version: number;
  key_id?: number;
  created_at?: string;
  created_by?: string;
  rotated_at?: string | null;
  owner_node?: string | null;
  owner_field?: string | null;
  deleted?: boolean;
  ciphertext_len?: number;
  revision?: unknown;
  /**
   * Present on `GET /{name}`: every version, newest first. The top-level
   * fields above are the newest version flattened, so this type reads as one
   * secret whether it came from the collection or the single-name endpoint.
   */
  versions?: SecretMetadata[];
}

/**
 * What a write returns. `version` is always present — a delete's tombstone is
 * itself a version, so `secret rm` can say which ordinal it wrote.
 * `reference` is the exact unpinned `secret://name` string to paste into a
 * node property, so the operator never has to assemble it by hand.
 */
export interface SecretWriteResult {
  name: string;
  version: number;
  reference?: string;
  deleted?: boolean;
}

export interface ScopeOptions {
  repo?: string;
  branch?: string;
}

/**
 * Resolve `{repo, branch}` for a secret operation.
 *
 * The repo is required in substance — a secret written to the wrong repo is
 * invisible to the function that needs it — so an unresolvable repo is an
 * error naming the two ways to supply it, never a silent fallback to
 * "default".
 */
export function resolveScope(
  options: ScopeOptions,
  defaultRepoImpl: () => string | null = getDefaultRepo
): { repo: string; branch: string } {
  const repo = options.repo || defaultRepoImpl();
  if (!repo) {
    throw new Error(
      'No repository specified. Pass --repo <name> or set a default with `raisindb repo use`.'
    );
  }
  return { repo, branch: options.branch || DEFAULT_BRANCH };
}

/** The `/api/secrets/{repo}/{branch}` base path, each segment encoded. */
export function secretsBasePath(repo: string, branch: string): string {
  return `/api/secrets/${encodeURIComponent(repo)}/${encodeURIComponent(branch)}`;
}

/**
 * Encode a secret name for a URL path, preserving `/` as a path separator.
 *
 * The server captures the name with a wildcard precisely BECAUSE it may contain
 * `/` (`node/{id}/{field.path}`), so the slashes must reach it as real
 * separators. Everything else in each segment is percent-encoded, which is what
 * keeps a `#`, `?` or space in a name from truncating the request.
 */
export function encodeSecretName(name: string): string {
  return name.split('/').map(encodeURIComponent).join('/');
}

/** `POST .../rotate/{name}` — literal segment first; see the module header. */
export function secretRotatePath(repo: string, branch: string, name: string): string {
  return `${secretsBasePath(repo, branch)}/rotate/${encodeSecretName(name)}`;
}

/** `{PUT,GET,DELETE} .../{name}` */
export function secretNamePath(repo: string, branch: string, name: string): string {
  return `${secretsBasePath(repo, branch)}/${encodeSecretName(name)}`;
}

/**
 * Reject names the store would reject anyway, but with a message that says
 * which rule was broken. Mirrors `raisin_models::secret_ref::validate_secret_name`:
 * non-empty, no NUL, no surrounding whitespace.
 */
export function validateSecretName(name: string): string {
  if (!name || !name.trim()) {
    throw new Error('Secret name must not be empty.');
  }
  if (name !== name.trim()) {
    throw new Error(`Invalid secret name '${name}': leading/trailing whitespace is not allowed.`);
  }
  if (name.includes('\0')) {
    throw new Error('Invalid secret name: NUL bytes are not allowed.');
  }
  return name;
}

export interface ValueFlags {
  /** Explicit value. Discouraged: it lands in shell history and `ps` output. */
  value?: string;
  /** Read the value from an environment variable. */
  valueEnv?: string;
}

export interface ValueDeps {
  env?: Record<string, string | undefined>;
  readStdinImpl?: () => Promise<string>;
}

/**
 * Resolve the secret value. STDIN is the DEFAULT — no flag needed — because
 * the alternative that feels most natural (`secret set NAME value`) is exactly
 * the one that writes the credential to shell history and exposes it in the
 * process table.
 *
 * The value never appears in an error message here.
 */
export async function resolveSecretValue(
  flags: ValueFlags = {},
  deps: ValueDeps = {}
): Promise<{ value: string; source: string }> {
  const sources = [
    flags.value !== undefined ? '--value' : null,
    flags.valueEnv !== undefined ? '--value-env' : null,
  ].filter((s): s is string => s !== null);

  if (sources.length > 1) {
    throw new Error(`${sources.join(' and ')} are mutually exclusive - provide the value one way.`);
  }

  if (flags.value !== undefined) {
    if (!flags.value) {
      throw new Error('--value was given an empty value.');
    }
    return { value: flags.value, source: 'flag' };
  }

  if (flags.valueEnv !== undefined) {
    const env = deps.env ?? process.env;
    const raw = env[flags.valueEnv];
    if (!raw || !raw.trim()) {
      throw new Error(`Environment variable ${flags.valueEnv} is not set or empty.`);
    }
    return { value: raw.trim(), source: `env:${flags.valueEnv}` };
  }

  const stdinReader = deps.readStdinImpl ?? readStdin;
  const value = await stdinReader();
  if (!value) {
    throw new Error(
      'No secret value received on stdin. Pipe it in (echo -n "value" | raisindb secret set NAME) ' +
        'or use --value-env VAR.'
    );
  }
  return { value, source: 'stdin' };
}

/** Rows for `secret list`, newest-version-first metadata per name. */
export function formatSecretTable(secrets: SecretMetadata[]): string {
  return formatTable(
    ['NAME', 'VERSION', 'UPDATED', 'BY', 'STATE'],
    secrets.map((s) => [
      s.name,
      String(s.version ?? ''),
      s.rotated_at || s.created_at || '',
      s.created_by || '',
      // A tombstoned name is shown rather than hidden: an operator needs to see
      // that a name was retired, not conclude it never existed.
      s.deleted ? 'deleted' : 'active',
    ])
  );
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export interface SecretSetOptions extends ScopeOptions, ValueFlags {}

export async function secretSet(
  name: string,
  options: SecretSetOptions = {},
  fetchImpl?: FetchLike,
  valueDeps?: ValueDeps
): Promise<void> {
  validateSecretName(name);
  const { repo, branch } = resolveScope(options);
  const resolved = await resolveSecretValue(options, valueDeps);

  const result = await apiCall<SecretWriteResult>(secretNamePath(repo, branch, name), {
    method: 'PUT',
    body: { value: resolved.value },
    fetchImpl,
  });

  if (!result.ok) {
    throw new Error(`Failed to set secret '${name}' in ${repo}/${branch}: ${result.errorMessage}`);
  }

  const version = result.data?.version;
  console.log(
    `Secret '${name}' written to ${repo}/${branch}` +
      (version !== undefined ? ` as version ${version}` : '') +
      ` (value from ${resolved.source}).`
  );
  printReference(result.data);
}

/**
 * Print the `secret://` reference a write returned, so the operator can paste
 * it straight into a node property instead of assembling it themselves (and
 * getting the `@version` rule wrong). Unpinned by design: a property that
 * pins a version stops picking up rotations.
 */
function printReference(data: SecretWriteResult | null): void {
  const reference = data?.reference;
  if (reference) {
    console.log(`  Reference (paste into an encrypted property): ${reference}`);
  }
}

export interface SecretListOptions extends ScopeOptions {
  json?: boolean;
}

export async function secretList(
  options: SecretListOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  const { repo, branch } = resolveScope(options);

  const result = await apiCall<{ secrets: SecretMetadata[] }>(secretsBasePath(repo, branch), {
    fetchImpl,
  });
  if (!result.ok || !result.data) {
    throw new Error(`Failed to list secrets in ${repo}/${branch}: ${result.errorMessage}`);
  }

  const secrets = result.data.secrets ?? [];

  if (options.json) {
    console.log(JSON.stringify(secrets, null, 2));
    return;
  }
  if (secrets.length === 0) {
    console.log(`No secrets in ${repo}/${branch}.`);
    return;
  }
  console.log(formatSecretTable(secrets));
}

export interface SecretShowOptions extends ScopeOptions {
  json?: boolean;
}

/**
 * Metadata for one secret. There is no plaintext in this output and no flag
 * that would add one — see the module header.
 */
export async function secretShow(
  name: string,
  options: SecretShowOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  validateSecretName(name);
  const { repo, branch } = resolveScope(options);

  const result = await apiCall<SecretMetadata>(secretNamePath(repo, branch, name), { fetchImpl });
  if (!result.ok || !result.data) {
    if (result.status === 404) {
      throw new Error(`Secret '${name}' not found in ${repo}/${branch}.`);
    }
    throw new Error(`Failed to read secret '${name}' in ${repo}/${branch}: ${result.errorMessage}`);
  }

  if (options.json) {
    console.log(JSON.stringify(result.data, null, 2));
    return;
  }
  // The body flattens the newest version to the top level AND carries every
  // version, so the table shows the full history without a second request.
  const versions = result.data.versions?.length ? result.data.versions : [result.data];
  console.log(formatSecretTable(versions));
}

export interface SecretRotateOptions extends ScopeOptions, ValueFlags {}

export async function secretRotate(
  name: string,
  options: SecretRotateOptions = {},
  fetchImpl?: FetchLike,
  valueDeps?: ValueDeps
): Promise<void> {
  validateSecretName(name);
  const { repo, branch } = resolveScope(options);
  const resolved = await resolveSecretValue(options, valueDeps);

  const result = await apiCall<SecretWriteResult>(secretRotatePath(repo, branch, name), {
    method: 'POST',
    body: { value: resolved.value },
    fetchImpl,
  });

  if (!result.ok) {
    throw new Error(
      `Failed to rotate secret '${name}' in ${repo}/${branch}: ${result.errorMessage}`
    );
  }

  const version = result.data?.version;
  console.log(
    `Secret '${name}' rotated in ${repo}/${branch}` +
      (version !== undefined ? ` to version ${version}` : '') +
      ` (value from ${resolved.source}). Pinned secret://${name}@N references still resolve.`
  );
  printReference(result.data);
}

export interface SecretRemoveOptions extends ScopeOptions {
  yes?: boolean;
}

export async function secretRemove(
  name: string,
  options: SecretRemoveOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  validateSecretName(name);
  const { repo, branch } = resolveScope(options);

  if (!options.yes) {
    throw new Error(
      `Refusing to delete secret '${name}' without --yes. Anything reading it will start failing.`
    );
  }

  const result = await apiCall<SecretWriteResult>(secretNamePath(repo, branch, name), {
    method: 'DELETE',
    fetchImpl,
  });

  if (!result.ok) {
    if (result.status === 404) {
      throw new Error(`Secret '${name}' not found in ${repo}/${branch}.`);
    }
    throw new Error(
      `Failed to delete secret '${name}' in ${repo}/${branch}: ${result.errorMessage}`
    );
  }

  const version = result.data?.version;
  console.log(
    `Secret '${name}' deleted in ${repo}/${branch}` +
      // The tombstone is itself a version, so there is always an ordinal.
      (version !== undefined ? ` (tombstone is version ${version})` : '') +
      '. Earlier versions remain readable through a pinned secret://name@N reference.'
  );
}
