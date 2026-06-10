import { readStdin } from './admin-util.js';

/**
 * Pure types + logic for `raisindb ai provider ...` (testable without a
 * server): --model parsing, API key source resolution, and the
 * read-modify-write merge for PUT /api/tenants/{tenant}/ai/config.
 */

/** serde names of AIProvider in raisin-ai */
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
  provider: string;
  has_api_key?: boolean;
  api_endpoint?: string | null;
  enabled: boolean;
  models: AIModelConfig[];
}

export interface AIConfigResponse {
  tenant_id: string;
  providers: ProviderState[];
  embedding_settings?: unknown;
}

/** Provider entry for the PUT body (api_key_plain only when changing it). */
export interface ProviderPutEntry {
  provider: string;
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
  apiKey?: string;
  endpoint?: string;
  /** undefined = keep existing (new providers default to enabled) */
  enabled?: boolean;
  /** undefined = keep existing models */
  models?: AIModelConfig[];
}

/**
 * Read-modify-write merge: keep all other providers untouched (their stored
 * keys are preserved server-side because api_key_plain is omitted), and
 * replace/insert only the named provider. embedding_settings is passed
 * through because PUT replaces the whole config document.
 */
export function mergeProviderConfig(
  current: AIConfigResponse,
  provider: string,
  opts: MergeOptions
): SetConfigBody {
  const existing = current.providers.find((p) => p.provider === provider);

  const others: ProviderPutEntry[] = current.providers
    .filter((p) => p.provider !== provider)
    .map((p) => {
      const entry: ProviderPutEntry = {
        provider: p.provider,
        enabled: p.enabled,
        models: p.models ?? [],
      };
      if (p.api_endpoint) {
        entry.api_endpoint = p.api_endpoint;
      }
      return entry;
    });

  const target: ProviderPutEntry = {
    provider,
    enabled: opts.enabled ?? existing?.enabled ?? true,
    models: opts.models ?? existing?.models ?? [],
  };
  const endpoint = opts.endpoint ?? existing?.api_endpoint ?? undefined;
  if (endpoint) {
    target.api_endpoint = endpoint;
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
