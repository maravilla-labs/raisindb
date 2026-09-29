import { apiCall, formatTable, FetchLike } from './admin-util.js';

/**
 * `raisindb repo ...` - repository administration over the HTTP API.
 *
 * Endpoints (raisin-transport-http):
 *   POST   /api/repositories            {repo_id, description?, default_language?, supported_languages?}
 *   GET    /api/repositories
 *   DELETE /api/repositories/{repo_id}
 *   GET    /api/repositories/{repo_id}/translation-config
 *   PATCH  /api/repositories/{repo_id}/translation-config  {supported_languages}
 *
 * A repository's `default_language` is fixed when it is created: the server
 * never changes it afterwards, because content in the default language is the
 * base that translation overlays are stored against. Only the list of
 * supported (translation) languages can grow later.
 */

interface RepositoryInfo {
  tenant_id: string;
  repo_id: string;
  created_at: string;
  branches?: string[];
  config?: {
    default_branch?: string;
    description?: string;
    default_language?: string;
    supported_languages?: string[];
  };
}

interface TranslationConfig {
  default_language: string;
  supported_languages: string[];
  locale_fallback_chains?: Record<string, string[]>;
}

/** Split `de,fr, en` into `['de', 'fr', 'en']`, dropping empties and duplicates. */
export function parseLanguageList(value: string | undefined): string[] {
  if (!value) return [];
  const out: string[] = [];
  for (const part of value.split(',')) {
    const code = part.trim();
    if (code && !out.includes(code)) out.push(code);
  }
  return out;
}

/**
 * Put the default language first and make sure it is supported: the server
 * adds it anyway, but echoing the list the user will actually get is clearer.
 */
function withDefaultFirst(defaultLanguage: string, languages: string[]): string[] {
  return [defaultLanguage, ...languages.filter((l) => l !== defaultLanguage)];
}

function describeLanguages(config: { default_language?: string; supported_languages?: string[] }): string {
  const def = config.default_language ?? 'en';
  const others = (config.supported_languages ?? []).filter((l) => l !== def);
  return others.length > 0 ? `${def} (default), ${others.join(', ')}` : `${def} (default)`;
}

export interface RepoCreateOptions {
  description?: string;
  /** Exit successfully if the repository already exists (idempotent CI). */
  existsOk?: boolean;
  /** Base language of the content. Immutable after creation; server default is `en`. */
  defaultLanguage?: string;
  /** Comma-separated translation languages, e.g. `de,fr,en`. */
  languages?: string;
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

  const languages = parseLanguageList(options.languages);
  const defaultLanguage = options.defaultLanguage?.trim() || undefined;
  if (languages.length > 0 && !defaultLanguage) {
    // Without an explicit default the server picks `en`, and the first entry of
    // --languages is what people mean by "the base language". Refuse to guess
    // about something that can never be changed afterwards.
    throw new Error(
      `--languages requires --default-language: the default language cannot be changed after ` +
        `the repository is created.\n` +
        `For example: raisindb repo create ${name} --default-language ${languages[0]} --languages ${languages.join(',')}`
    );
  }
  if (defaultLanguage) {
    body.default_language = defaultLanguage;
    body.supported_languages = withDefaultFirst(defaultLanguage, languages);
  }

  const result = await apiCall<RepositoryInfo>('/api/repositories', {
    method: 'POST',
    body,
    fetchImpl,
  });

  if (result.ok) {
    const config = result.data?.config ?? {
      default_language: defaultLanguage,
      supported_languages: body.supported_languages as string[] | undefined,
    };
    console.log(`Repository '${name}' created. Languages: ${describeLanguages(config)}.`);
    return;
  }

  if (result.status === 409) {
    if (options.existsOk) {
      if (defaultLanguage || languages.length > 0) {
        await checkExistingLanguages(name, defaultLanguage, languages, fetchImpl);
      }
      console.log(`Repository '${name}' already exists (ok).`);
      return;
    }
    throw new Error(`Repository '${name}' already exists (use --exists-ok to ignore).`);
  }

  throw new Error(`Failed to create repository '${name}': ${result.errorMessage}`);
}

/**
 * `--exists-ok` must not hide a repository whose languages differ from what
 * the caller asked for: a wrong default language cannot be fixed later, so
 * that is an error; missing translation languages only need `repo languages --add`.
 */
async function checkExistingLanguages(
  name: string,
  defaultLanguage: string | undefined,
  languages: string[],
  fetchImpl?: FetchLike
): Promise<void> {
  const current = await apiCall<TranslationConfig>(
    `/api/repositories/${encodeURIComponent(name)}/translation-config`,
    { fetchImpl }
  );
  if (!current.ok || !current.data) {
    return;
  }
  const { default_language, supported_languages } = current.data;
  if (defaultLanguage && default_language !== defaultLanguage) {
    throw new Error(
      `Repository '${name}' already exists with default language '${default_language}', ` +
        `not '${defaultLanguage}'. The default language cannot be changed after creation; ` +
        `delete and recreate the repository to change it.`
    );
  }
  const missing = languages.filter((l) => !supported_languages.includes(l));
  if (missing.length > 0) {
    console.log(
      `Warning: repository '${name}' does not support ${missing.join(', ')}. ` +
        `Add them with: raisindb repo languages ${name} --add ${missing.join(',')}`
    );
  }
}

export interface RepoLanguagesOptions {
  /** Comma-separated languages to add to supported_languages. */
  add?: string;
  json?: boolean;
}

/**
 * Show a repository's languages, or add translation languages with `--add`.
 * The default language is shown but can never be changed.
 */
export async function repoLanguages(
  name: string,
  options: RepoLanguagesOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  const path = `/api/repositories/${encodeURIComponent(name)}/translation-config`;
  const current = await apiCall<TranslationConfig>(path, { fetchImpl });
  if (current.status === 404) {
    throw new Error(`Repository '${name}' not found.`);
  }
  if (!current.ok || !current.data) {
    throw new Error(`Failed to read languages of '${name}': ${current.errorMessage}`);
  }

  let config = current.data;
  const toAdd = parseLanguageList(options.add).filter((l) => !config.supported_languages.includes(l));

  if (toAdd.length > 0) {
    const updated = await apiCall<unknown>(path, {
      method: 'PATCH',
      body: { supported_languages: [...config.supported_languages, ...toAdd] },
      fetchImpl,
    });
    if (!updated.ok) {
      throw new Error(`Failed to update languages of '${name}': ${updated.errorMessage}`);
    }
    // Re-read so the output is what the server stored, not what we sent.
    const reread = await apiCall<TranslationConfig>(path, { fetchImpl });
    if (reread.ok && reread.data) {
      config = reread.data;
    }
    if (!options.json) {
      console.log(`Added ${toAdd.join(', ')} to '${name}'.`);
    }
  } else if (options.add && !options.json) {
    console.log(`No change: '${name}' already supports ${parseLanguageList(options.add).join(', ')}.`);
  }

  if (options.json) {
    console.log(JSON.stringify(config, null, 2));
    return;
  }
  console.log(`Default language:    ${config.default_language} (fixed at creation)`);
  console.log(`Supported languages: ${config.supported_languages.join(', ')}`);
  const chains = Object.entries(config.locale_fallback_chains ?? {});
  if (chains.length > 0) {
    console.log('Fallback chains:');
    for (const [locale, chain] of chains) {
      console.log(`  ${locale} -> ${chain.join(' -> ')}`);
    }
  }
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
      ['REPO', 'DEFAULT BRANCH', 'LANGUAGES', 'CREATED', 'DESCRIPTION'],
      repos.map((r) => [
        r.repo_id,
        r.config?.default_branch ?? 'main',
        describeLanguages(r.config ?? {}),
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
