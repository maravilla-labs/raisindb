import { apiCall, formatTable, FetchLike } from './admin-util.js';

/**
 * `raisindb repo ...` - repository administration over the HTTP API.
 *
 * Endpoints (raisin-transport-http):
 *   POST   /api/repositories            {repo_id, description?}
 *   GET    /api/repositories
 *   DELETE /api/repositories/{repo_id}
 */

interface RepositoryInfo {
  tenant_id: string;
  repo_id: string;
  created_at: string;
  branches?: string[];
  config?: {
    default_branch?: string;
    description?: string;
  };
}

export interface RepoCreateOptions {
  description?: string;
  /** Exit successfully if the repository already exists (idempotent CI). */
  existsOk?: boolean;
}

export async function repoCreate(
  name: string,
  options: RepoCreateOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  const body: Record<string, unknown> = { repo_id: name };
  if (options.description) {
    body.description = options.description;
  }

  const result = await apiCall<RepositoryInfo>('/api/repositories', {
    method: 'POST',
    body,
    fetchImpl,
  });

  if (result.ok) {
    console.log(`Repository '${name}' created.`);
    return;
  }

  if (result.status === 409) {
    if (options.existsOk) {
      console.log(`Repository '${name}' already exists (ok).`);
      return;
    }
    throw new Error(`Repository '${name}' already exists (use --exists-ok to ignore).`);
  }

  throw new Error(`Failed to create repository '${name}': ${result.errorMessage}`);
}

export interface RepoListOptions {
  json?: boolean;
}

export async function repoList(options: RepoListOptions = {}, fetchImpl?: FetchLike): Promise<void> {
  const result = await apiCall<RepositoryInfo[]>('/api/repositories', { fetchImpl });

  if (!result.ok || !result.data) {
    throw new Error(`Failed to list repositories: ${result.errorMessage}`);
  }

  const repos = result.data;

  if (options.json) {
    console.log(JSON.stringify(repos, null, 2));
    return;
  }

  if (repos.length === 0) {
    console.log('No repositories found.');
    return;
  }

  console.log(
    formatTable(
      ['REPO', 'DEFAULT BRANCH', 'CREATED', 'DESCRIPTION'],
      repos.map((r) => [
        r.repo_id,
        r.config?.default_branch ?? 'main',
        r.created_at ?? '',
        r.config?.description ?? '',
      ])
    )
  );
}

export interface RepoDeleteOptions {
  /** Required confirmation flag - deletion is irreversible. */
  yes?: boolean;
}

export async function repoDelete(
  name: string,
  options: RepoDeleteOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  if (!options.yes) {
    throw new Error(
      `Deleting a repository removes ALL branches, revisions and nodes and cannot be undone.\n` +
        `Re-run with --yes to confirm: raisindb repo delete ${name} --yes`
    );
  }

  const result = await apiCall<unknown>(`/api/repositories/${encodeURIComponent(name)}`, {
    method: 'DELETE',
    fetchImpl,
  });

  if (result.ok) {
    console.log(`Repository '${name}' deleted.`);
    return;
  }

  if (result.status === 404) {
    throw new Error(`Repository '${name}' not found.`);
  }

  throw new Error(`Failed to delete repository '${name}': ${result.errorMessage}`);
}
