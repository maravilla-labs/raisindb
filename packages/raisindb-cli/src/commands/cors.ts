import { apiCall, FetchLike } from './admin-util.js';

/**
 * `raisindb cors ...` - manage CORS allowed origins.
 *
 * The server resolves CORS hierarchically (middleware/cors.rs):
 *   repo-level > tenant-level > global server config.
 *
 * Repo-level (default): a raisin:RepoAuthConfig node at
 * /config/repos/{repo} in the repo's raisin:system workspace, written via
 * the generic node API (same path the admin console uses):
 *   GET/PUT  /api/repository/{repo}/main/head/raisin:system/config/repos/{repo}
 *   POST     /api/repository/{repo}/main/head/raisin:system/config/repos
 *
 * Tenant-level (--tenant-level): TenantAuthConfig.cors_allowed_origins via
 *   GET/PUT  /api/tenants/{tenant}/auth/config
 */

export interface CorsOptions {
  repo?: string;
  tenant?: string;
  tenantLevel?: boolean;
  json?: boolean;
}

/** Validate that `origin` looks like a scheme://host[:port] origin (no path). */
export function validateOrigin(origin: string): string {
  const trimmed = origin.trim().replace(/\/$/, '');
  if (trimmed === '*') {
    return trimmed;
  }
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    throw new Error(`Invalid origin '${origin}' (expected e.g. http://localhost:5173 or *).`);
  }
  if (url.pathname !== '/' || url.search || url.hash) {
    throw new Error(`Invalid origin '${origin}': origins must not contain a path/query/fragment.`);
  }
  return url.origin;
}

export function addOrigin(origins: string[], origin: string): { origins: string[]; changed: boolean } {
  if (origins.includes(origin)) {
    return { origins, changed: false };
  }
  return { origins: [...origins, origin], changed: true };
}

export function removeOrigin(origins: string[], origin: string): { origins: string[]; changed: boolean } {
  if (!origins.includes(origin)) {
    return { origins, changed: false };
  }
  return { origins: origins.filter((o) => o !== origin), changed: true };
}

// ---------------------------------------------------------------------------
// Storage adapters (repo-level node vs tenant-level auth config)
// ---------------------------------------------------------------------------

interface NodeResponse {
  properties?: { cors_allowed_origins?: string[] };
}

function repoNodePath(repo: string): string {
  return `/api/repository/${encodeURIComponent(repo)}/main/head/raisin:system/config/repos/${encodeURIComponent(repo)}`;
}

async function readRepoOrigins(repo: string, fetchImpl?: FetchLike): Promise<string[] | null> {
  const result = await apiCall<NodeResponse>(repoNodePath(repo), { fetchImpl });
  if (result.status === 404) {
    return null; // RepoAuthConfig node does not exist yet
  }
  if (!result.ok) {
    throw new Error(`Failed to read repo CORS config for '${repo}': ${result.errorMessage}`);
  }
  return result.data?.properties?.cors_allowed_origins ?? [];
}

async function writeRepoOrigins(
  repo: string,
  origins: string[],
  nodeExists: boolean,
  fetchImpl?: FetchLike
): Promise<void> {
  const commit = { message: 'Update CORS allowed origins', actor: 'raisindb-cli' };

  if (nodeExists) {
    const result = await apiCall<unknown>(repoNodePath(repo), {
      method: 'PUT',
      body: { properties: { cors_allowed_origins: origins }, commit },
      fetchImpl,
    });
    if (!result.ok) {
      throw new Error(`Failed to update repo CORS config for '${repo}': ${result.errorMessage}`);
    }
    return;
  }

  // Create the raisin:RepoAuthConfig node (ensure parent folders exist first)
  const base = `/api/repository/${encodeURIComponent(repo)}/main/head/raisin:system`;
  const folders: Array<{ parent: string; name: string }> = [
    { parent: `${base}/`, name: 'config' },
    { parent: `${base}/config`, name: 'repos' },
  ];
  for (const folder of folders) {
    const result = await apiCall<unknown>(folder.parent, {
      method: 'POST',
      body: { name: folder.name, node_type: 'raisin:Folder', properties: {}, commit },
      fetchImpl,
    });
    // 409/conflict means the folder already exists - fine
    if (!result.ok && result.status !== 409) {
      throw new Error(
        `Failed to ensure folder '${folder.name}' in raisin:system for '${repo}': ${result.errorMessage}`
      );
    }
  }

  const create = await apiCall<unknown>(`${base}/config/repos`, {
    method: 'POST',
    body: {
      name: repo,
      node_type: 'raisin:RepoAuthConfig',
      properties: { repo_id: repo, cors_allowed_origins: origins },
      commit,
    },
    fetchImpl,
  });
  if (!create.ok) {
    throw new Error(`Failed to create repo CORS config for '${repo}': ${create.errorMessage}`);
  }
}

interface TenantAuthConfigResponse {
  cors_allowed_origins?: string[];
}

async function readTenantOrigins(tenant: string, fetchImpl?: FetchLike): Promise<string[]> {
  const result = await apiCall<TenantAuthConfigResponse>(
    `/api/tenants/${encodeURIComponent(tenant)}/auth/config`,
    { fetchImpl }
  );
  if (!result.ok) {
    throw new Error(`Failed to read tenant auth config for '${tenant}': ${result.errorMessage}`);
  }
  return result.data?.cors_allowed_origins ?? [];
}

async function writeTenantOrigins(tenant: string, origins: string[], fetchImpl?: FetchLike): Promise<void> {
  // PUT only updates the provided fields (handler merges into existing config)
  const result = await apiCall<unknown>(`/api/tenants/${encodeURIComponent(tenant)}/auth/config`, {
    method: 'PUT',
    body: { cors_allowed_origins: origins },
    fetchImpl,
  });
  if (!result.ok) {
    throw new Error(`Failed to update tenant auth config for '${tenant}': ${result.errorMessage}`);
  }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

function describeScope(options: CorsOptions): string {
  return options.tenantLevel
    ? `tenant '${options.tenant || 'default'}'`
    : `repo '${options.repo}'`;
}

function requireScope(options: CorsOptions): void {
  if (!options.tenantLevel && !options.repo) {
    throw new Error('--repo <name> is required (or use --tenant-level for tenant-wide CORS).');
  }
}

async function readOrigins(options: CorsOptions, fetchImpl?: FetchLike): Promise<string[]> {
  if (options.tenantLevel) {
    return readTenantOrigins(options.tenant || 'default', fetchImpl);
  }
  return (await readRepoOrigins(options.repo!, fetchImpl)) ?? [];
}

export async function corsAdd(origin: string, options: CorsOptions, fetchImpl?: FetchLike): Promise<void> {
  requireScope(options);
  const normalized = validateOrigin(origin);

  if (options.tenantLevel) {
    const tenant = options.tenant || 'default';
    const current = await readTenantOrigins(tenant, fetchImpl);
    const { origins, changed } = addOrigin(current, normalized);
    if (!changed) {
      console.log(`Origin '${normalized}' already allowed for ${describeScope(options)}.`);
      return;
    }
    await writeTenantOrigins(tenant, origins, fetchImpl);
  } else {
    const current = await readRepoOrigins(options.repo!, fetchImpl);
    const { origins, changed } = addOrigin(current ?? [], normalized);
    if (current !== null && !changed) {
      console.log(`Origin '${normalized}' already allowed for ${describeScope(options)}.`);
      return;
    }
    await writeRepoOrigins(options.repo!, origins, current !== null, fetchImpl);
  }
  console.log(`Origin '${normalized}' added to ${describeScope(options)} CORS allow-list.`);
}

export async function corsRemove(origin: string, options: CorsOptions, fetchImpl?: FetchLike): Promise<void> {
  requireScope(options);
  const normalized = validateOrigin(origin);
  const current = await readOrigins(options, fetchImpl);
  const { origins, changed } = removeOrigin(current, normalized);

  if (!changed) {
    throw new Error(`Origin '${normalized}' is not in the ${describeScope(options)} CORS allow-list.`);
  }

  if (options.tenantLevel) {
    await writeTenantOrigins(options.tenant || 'default', origins, fetchImpl);
  } else {
    await writeRepoOrigins(options.repo!, origins, true, fetchImpl);
  }
  console.log(`Origin '${normalized}' removed from ${describeScope(options)} CORS allow-list.`);
}

export async function corsList(options: CorsOptions, fetchImpl?: FetchLike): Promise<void> {
  requireScope(options);
  const origins = await readOrigins(options, fetchImpl);

  if (options.json) {
    console.log(JSON.stringify(origins, null, 2));
    return;
  }
  if (origins.length === 0) {
    console.log(`No CORS origins configured for ${describeScope(options)}.`);
    return;
  }
  console.log(`CORS allowed origins for ${describeScope(options)}:`);
  for (const o of origins) {
    console.log(`  ${o}`);
  }
}
