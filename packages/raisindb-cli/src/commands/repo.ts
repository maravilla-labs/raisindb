import { apiCall, formatTable, FetchLike } from './admin-util.js';

/**
 * `raisindb repo ...` - repository administration over the HTTP API.
 *
 * Endpoints (raisin-transport-http):
 *   POST   /api/repositories            {repo_id, description?, default_language?, supported_languages?}
 *   GET    /api/repositories
 *   DELETE /api/repositories/{repo_id}
 *   GET    /api/repositories/{repo_id}/translation-config
 *   PATCH  /api/repositories/{repo_id}/translation-config  {supported_languages?, default_language?}
 *
 * A repository's `default_language` is the language base content is stored in;
 * every other language is a translation overlay against it. Pick it at create
 * time. It can be changed later (`repo languages --default`): the server
 * refuses while translations in the new language exist, and queues a full-text
 * rebuild of every branch so search files base content under the new language.
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
  /** Base language of the content; server default is `en`. Change later with `repo languages --default`. */
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
    // --languages is what people mean by "the base language". Refuse to guess:
    // changing the default later means re-indexing the repository.
    throw new Error(
      `--languages requires --default-language: it is the language the base content is stored in.\n` +
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
 * the caller asked for: a different default language is an error (changing it
 * is a deliberate, re-indexing `repo languages --default`); missing translation
 * languages only need `repo languages --add`.
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
        `not '${defaultLanguage}'. To change it (this re-indexes the repository): ` +
        `raisindb repo languages ${name} --default ${defaultLanguage}`
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
  /** Make this language the default (base) language. Queues a full-text rebuild. */
  default?: string;
  /** Skip the confirmation for --default. */
  yes?: boolean;
  json?: boolean;
  /**
   * Ask the user to confirm a default-language change. Only set when a person
   * is at the terminal; without it, --default requires --yes.
   */
  confirm?: (question: string) => Promise<boolean>;
}

interface ReindexJob {
  branch: string;
  job_id: string;
}

interface UpdateTranslationConfigResponse extends TranslationConfig {
  previous_default_language?: string;
  reindex_jobs?: ReindexJob[];
}

interface DefaultLanguageConflict {
  message?: string;
  language?: string;
  overlay_count?: number;
}

/** What changing the default language does, shown before it is confirmed. */
export function describeDefaultLanguageChange(name: string, from: string, to: string): string {
  return (
    `Changing the default language of '${name}' from '${from}' to '${to}':\n` +
    `  - Base (untranslated) content is from now on treated as '${to}'. The content\n` +
    `    itself is not translated or rewritten.\n` +
    `  - '${to}' is added to the supported languages; '${from}' stays in them.\n` +
    `  - A full-text rebuild of every branch is queued, so search finds the base\n` +
    `    content under '${to}' instead of '${from}'. Until it finishes, full-text\n` +
    `    search may miss base content. Vector embeddings are not affected.\n` +
    `  - The server refuses the change while translations in '${to}' exist.`
  );
}

/**
 * Show a repository's languages, add translation languages with `--add`, or
 * change the default language with `--default` (confirmed, it queues a rebuild).
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
  const newDefault = options.default?.trim() || undefined;
  const changeDefault = newDefault !== undefined && newDefault !== config.default_language;

  if (newDefault !== undefined && !changeDefault && !options.json) {
    console.log(`No change: '${newDefault}' already is the default language of '${name}'.`);
  }

  if (changeDefault) {
    const explanation = describeDefaultLanguageChange(name, config.default_language, newDefault!);
    if (!options.yes) {
      if (!options.confirm) {
        throw new Error(`${explanation}\n\nRe-run with --yes to confirm: raisindb repo languages ${name} --default ${newDefault} --yes`);
      }
      console.log(explanation);
      if (!(await options.confirm('Change the default language? [y/N] '))) {
        console.log('Aborted; nothing changed.');
        return;
      }
    }
  }

  if (toAdd.length > 0 || changeDefault) {
    const body: Record<string, unknown> = {};
    if (toAdd.length > 0) body.supported_languages = [...config.supported_languages, ...toAdd];
    if (changeDefault) body.default_language = newDefault;

    const updated = await apiCall<UpdateTranslationConfigResponse & DefaultLanguageConflict>(path, {
      method: 'PATCH',
      body,
      fetchImpl,
    });
    if (updated.status === 409 && changeDefault) {
      const count = updated.data?.overlay_count;
      throw new Error(
        `Cannot make '${newDefault}' the default language of '${name}': ` +
          `${count ?? 'some'} translation(s) in '${newDefault}' already exist and would collide ` +
          `with the base content. Delete those translations first. Nothing was changed.`
      );
    }
    if (!updated.ok) {
      throw new Error(`Failed to update languages of '${name}': ${updated.errorMessage}`);
    }

    const jobs = updated.data?.reindex_jobs ?? [];
    // Re-read so the output is what the server stored, not what we sent.
    const reread = await apiCall<TranslationConfig>(path, { fetchImpl });
    if (reread.ok && reread.data) {
      config = reread.data;
    }
    if (!options.json) {
      if (toAdd.length > 0) {
        console.log(`Added ${toAdd.join(', ')} to '${name}'.`);
      }
      if (changeDefault) {
        console.log(
          `Default language of '${name}' changed from '${updated.data?.previous_default_language ?? current.data.default_language}' to '${config.default_language}'.`
        );
        for (const job of jobs) {
          console.log(`Queued full-text rebuild of branch '${job.branch}' (job ${job.job_id}).`);
        }
      }
    }
  } else if (options.add && !options.json) {
    console.log(`No change: '${name}' already supports ${parseLanguageList(options.add).join(', ')}.`);
  }

  if (options.json) {
    console.log(JSON.stringify(config, null, 2));
    return;
  }
  console.log(`Default language:    ${config.default_language}`);
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
