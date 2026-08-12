import { readStdin } from './admin-util.js';

/**
 * Pure types + logic for `raisindb ai provider ...` (testable without a
 * server): --model parsing, slug validation, API key source resolution, and
 * the read-modify-write merge for PUT /api/tenants/{tenant}/ai/config.
 *
 * Identity note: a provider entry is identified by its per-tenant `slug`, not
 * by its kind, so one tenant can configure several providers of the same kind
 * (three OpenAI-compatible gateways, say). The kind still travels on the wire
 * under the key `provider` - renaming that key would drop every stored
 * provider type - so this file consistently calls the wire field `provider`
 * and the concept "kind".
 */

/** serde names of AIProvider in raisin-ai - the *kinds* a slug can have */
export const KNOWN_PROVIDERS = [
  'openai',
  'anthropic',
  'google',
  'ollama',
  'azure_openai',
  'groq',
  'openrouter',
  'bedrock',
  'custom',
  'local',
] as const;

/**
 * A slug is lowercase alphanumerics and dashes, starting alphanumeric, at most
 * 39 characters. Mirrors the server-side rule exactly so a typo costs a local
 * error instead of a round trip and a 400.
 */
export const SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,38}$/;

/**
 * True when `name` is exactly the serde name of an AIProvider kind. Kind names
 * double as the legacy default slugs - a config written before slugs existed
 * gets `slug = kind` - so they are always legal slugs.
 */
export function isProviderKind(name: string): boolean {
  return (KNOWN_PROVIDERS as readonly string[]).includes(name);
}

/**
 * Validate a provider slug, throwing with the rule that was broken.
 * `:` gets its own message because it is the one character whose rejection
 * looks arbitrary until you know model ids are `<slug>:<model>`.
 *
 * A slug that is exactly a kind name is grandfathered in, on create as well as
 * update: `azure_openai` is the legacy default slug of the AzureOpenAI kind and
 * the admin console sends it on every save, yet the pattern rejects it for its
 * underscore. The server makes the same exception, and the two rules have to
 * stay identical - a stricter client refuses calls the API would accept.
 */
export function validateProviderSlug(slug: string): void {
  if (isProviderKind(slug)) {
    return;
  }
  if (slug.includes(':')) {
    throw new Error(
      `Invalid provider slug '${slug}': ':' is reserved as the separator in model ids ` +
        `(<slug>:<model>, e.g. 'marvel:gpt-4o') and can never appear in a slug.`
    );
  }
  if (!SLUG_PATTERN.test(slug)) {
    throw new Error(
      `Invalid provider slug '${slug}'. A slug is lowercase a-z, 0-9 and '-', ` +
        `must start with a letter or digit, and is at most 39 characters.`
    );
  }
}

export interface AIModelConfig {
  model_id: string;
  display_name: string;
  use_cases: string[];
  default_temperature: number;
  default_max_tokens: number;
  is_default: boolean;
  metadata?: unknown;
}

/** Provider entry as returned by GET /ai/config (no key material). */
export interface ProviderState {
  /**
   * Per-tenant identity, immutable once created. Optional only because a
   * server that predates slugs omits it - see entrySlug().
   */
  slug?: string;
  /** The provider KIND (openai, custom, ...); wire key stays `provider`. */
  provider: string;
  display_name?: string | null;
  icon_url?: string | null;
  has_api_key?: boolean;
  api_endpoint?: string | null;
  enabled: boolean;
  models: AIModelConfig[];
}

/**
 * Slug of a stored entry, applying the same shim the server does for configs
 * written before slugs existed: no slug means the slug *is* the kind, which is
 * what keeps model ids like `openai:gpt-4o` resolving unchanged. Servers that
 * already speak the new contract always send `slug`; this only matters when a
 * new CLI talks to a not-yet-migrated one.
 */
export function entrySlug(entry: ProviderState): string {
  return entry.slug || entry.provider;
}

export interface AIConfigResponse {
  tenant_id: string;
  providers: ProviderState[];
  embedding_settings?: unknown;
}

/** Provider entry for the PUT body (api_key_plain only when changing it). */
export interface ProviderPutEntry {
  slug: string;
  provider: string;
  display_name?: string;
  icon_url?: string;
  api_key_plain?: string;
  api_endpoint?: string;
  enabled: boolean;
  models: AIModelConfig[];
}

export interface SetConfigBody {
  providers: ProviderPutEntry[];
  embedding_settings?: unknown;
}

/**
 * Parse a --model spec: `model_id[:display_name]`.
 * The first model in the list becomes the default.
 */
export function parseModelSpec(spec: string, index: number): AIModelConfig {
  const trimmed = spec.trim();
  const colon = trimmed.indexOf(':');
  const modelId = colon === -1 ? trimmed : trimmed.slice(0, colon).trim();
  const displayName = colon === -1 ? trimmed : trimmed.slice(colon + 1).trim() || trimmed;

  if (!modelId) {
    throw new Error(`Invalid --model spec: "${spec}" (expected model_id[:display_name])`);
  }

  return {
    model_id: modelId,
    display_name: displayName,
    use_cases: ['chat', 'agent'],
    default_temperature: 0.3,
    default_max_tokens: 1024,
    is_default: index === 0,
  };
}

export interface ApiKeyFlags {
  apiKey?: string;
  apiKeyStdin?: boolean;
  apiKeyEnv?: string;
}

export interface ApiKeyDeps {
  env?: Record<string, string | undefined>;
  readStdinImpl?: () => Promise<string>;
}

/**
 * Resolve the API key from exactly one of --api-key / --api-key-stdin /
 * --api-key-env. Returns null when no key flag was given (key unchanged).
 * The key value itself never appears in error messages.
 */
export async function resolveApiKey(
  flags: ApiKeyFlags,
  deps: ApiKeyDeps = {}
): Promise<{ key: string; source: string } | null> {
  const sources = [
    flags.apiKey !== undefined ? '--api-key' : null,
    flags.apiKeyStdin ? '--api-key-stdin' : null,
    flags.apiKeyEnv !== undefined ? '--api-key-env' : null,
  ].filter((s): s is string => s !== null);

  if (sources.length === 0) {
    return null;
  }
  if (sources.length > 1) {
    throw new Error(`${sources.join(' and ')} are mutually exclusive - provide the key one way.`);
  }

  if (flags.apiKey !== undefined) {
    if (!flags.apiKey) {
      throw new Error('--api-key was given an empty value.');
    }
    return { key: flags.apiKey, source: 'flag' };
  }

  if (flags.apiKeyEnv !== undefined) {
    const env = deps.env ?? process.env;
    const value = env[flags.apiKeyEnv];
    if (!value || !value.trim()) {
      throw new Error(`Environment variable ${flags.apiKeyEnv} is not set or empty.`);
    }
    return { key: value.trim(), source: `env:${flags.apiKeyEnv}` };
  }

  const stdinReader = deps.readStdinImpl ?? readStdin;
  const value = (await stdinReader()).trim();
  if (!value) {
    throw new Error('No API key received on stdin.');
  }
  return { key: value, source: 'stdin' };
}

export interface MergeOptions {
  /** Provider kind; required when the slug is new, ignored when it exists. */
  kind?: string;
  apiKey?: string;
  endpoint?: string;
  displayName?: string;
  iconUrl?: string;
  /** undefined = keep existing (new providers default to enabled) */
  enabled?: boolean;
  /** undefined = keep existing models */
  models?: AIModelConfig[];
}

/**
 * Read-modify-write merge: keep all other providers untouched (their stored
 * keys are preserved server-side because api_key_plain is omitted), and
 * replace/insert only the entry with this slug. embedding_settings is passed
 * through because PUT replaces the whole config document.
 *
 * Matching is by slug, never by kind: two entries may share a kind, and
 * matching on kind would silently overwrite the sibling. The kind of an
 * existing slug is carried over rather than taken from opts, because a slug's
 * identity - and everything referencing it, model ids and `ai_provider_ref`
 * included - is only stable if it keeps pointing at the same thing.
 */
export function mergeProviderConfig(
  current: AIConfigResponse,
  slug: string,
  opts: MergeOptions
): SetConfigBody {
  const existing = current.providers.find((p) => entrySlug(p) === slug);

  const kind = existing?.provider ?? opts.kind;
  if (!kind) {
    throw new Error(
      `Provider '${slug}' does not exist yet - pass --kind <${KNOWN_PROVIDERS.join('|')}> to create it.`
    );
  }

  const others: ProviderPutEntry[] = current.providers
    .filter((p) => entrySlug(p) !== slug)
    .map((p) => {
      // Carry every stored field forward: PUT replaces the document, so a
      // field left out here is a field deleted from an untouched provider.
      const entry: ProviderPutEntry = {
        slug: entrySlug(p),
        provider: p.provider,
        enabled: p.enabled,
        models: p.models ?? [],
      };
      if (p.api_endpoint) {
        entry.api_endpoint = p.api_endpoint;
      }
      if (p.display_name) {
        entry.display_name = p.display_name;
      }
      if (p.icon_url) {
        entry.icon_url = p.icon_url;
      }
      return entry;
    });

  const target: ProviderPutEntry = {
    slug,
    provider: kind,
    enabled: opts.enabled ?? existing?.enabled ?? true,
    models: opts.models ?? existing?.models ?? [],
  };
  const endpoint = opts.endpoint ?? existing?.api_endpoint ?? undefined;
  if (endpoint) {
    target.api_endpoint = endpoint;
  }
  const displayName = opts.displayName ?? existing?.display_name ?? undefined;
  if (displayName) {
    target.display_name = displayName;
  }
  const iconUrl = opts.iconUrl ?? existing?.icon_url ?? undefined;
  if (iconUrl) {
    target.icon_url = iconUrl;
  }
  if (opts.apiKey !== undefined) {
    target.api_key_plain = opts.apiKey;
  }

  const body: SetConfigBody = { providers: [...others, target] };
  if (current.embedding_settings !== undefined && current.embedding_settings !== null) {
    body.embedding_settings = current.embedding_settings;
  }
  return body;
}
