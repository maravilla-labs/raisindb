import { apiCall, formatTable, FetchLike } from './admin-util.js';
import {
  AIConfigResponse,
  ApiKeyDeps,
  ApiKeyFlags,
  KNOWN_PROVIDERS,
  mergeProviderConfig,
  parseModelSpec,
  resolveApiKey,
} from './ai-config.js';

/**
 * `raisindb ai provider ...` - tenant AI provider configuration (gh-secret
 * style: the API key is never echoed, never logged).
 *
 * Endpoints (raisin-transport-http, handlers/ai/config.rs):
 *   GET  /api/tenants/{tenant}/ai/config
 *   PUT  /api/tenants/{tenant}/ai/config        (REPLACES the provider list;
 *        a provider's stored key is preserved when api_key_plain is omitted)
 *   GET  /api/tenants/{tenant}/ai/providers
 *   POST /api/tenants/{tenant}/ai/providers/{provider}/test
 */

export interface AiProviderSetCliOptions extends ApiKeyFlags {
  endpoint?: string;
  enabled?: boolean;
  disabled?: boolean;
  model?: string[];
  tenant?: string;
}

export async function aiProviderSet(
  provider: string,
  options: AiProviderSetCliOptions,
  fetchImpl?: FetchLike,
  keyDeps?: ApiKeyDeps
): Promise<void> {
  if (!(KNOWN_PROVIDERS as readonly string[]).includes(provider)) {
    throw new Error(
      `Unknown provider '${provider}'. Known providers: ${KNOWN_PROVIDERS.join(', ')}`
    );
  }
  if (options.enabled && options.disabled) {
    throw new Error('--enabled and --disabled are mutually exclusive.');
  }

  const tenant = options.tenant || 'default';
  const resolved = await resolveApiKey(options, keyDeps);
  const models = options.model?.length
    ? options.model.map((spec, i) => parseModelSpec(spec, i))
    : undefined;

  // Read current config (GET never exposes key material)
  const getResult = await apiCall<AIConfigResponse>(
    `/api/tenants/${encodeURIComponent(tenant)}/ai/config`,
    { fetchImpl }
  );
  if (!getResult.ok || !getResult.data) {
    throw new Error(`Failed to read AI config for tenant '${tenant}': ${getResult.errorMessage}`);
  }

  const enabled = options.enabled ? true : options.disabled ? false : undefined;
  const body = mergeProviderConfig(getResult.data, provider, {
    apiKey: resolved?.key,
    endpoint: options.endpoint,
    enabled,
    models,
  });

  const putResult = await apiCall<{ success: boolean; message?: string }>(
    `/api/tenants/${encodeURIComponent(tenant)}/ai/config`,
    { method: 'PUT', body, fetchImpl }
  );
  if (!putResult.ok) {
    throw new Error(`Failed to update AI config for tenant '${tenant}': ${putResult.errorMessage}`);
  }

  const targetEntry = body.providers[body.providers.length - 1];
  const details = [
    `enabled=${targetEntry.enabled}`,
    `models=${targetEntry.models.length}`,
    resolved ? `api_key=updated (from ${resolved.source})` : 'api_key=unchanged',
  ];
  console.log(`Provider '${provider}' configured for tenant '${tenant}' (${details.join(', ')}).`);
}

interface ProviderSummary {
  provider: string;
  enabled: boolean;
  has_api_key: boolean;
  model_count: number;
}

export interface AiProviderListOptions {
  tenant?: string;
  json?: boolean;
}

export async function aiProviderList(
  options: AiProviderListOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  const tenant = options.tenant || 'default';
  const result = await apiCall<{ providers: ProviderSummary[] }>(
    `/api/tenants/${encodeURIComponent(tenant)}/ai/providers`,
    { fetchImpl }
  );
  if (!result.ok || !result.data) {
    throw new Error(`Failed to list AI providers for tenant '${tenant}': ${result.errorMessage}`);
  }

  const providers = result.data.providers ?? [];

  if (options.json) {
    console.log(JSON.stringify(providers, null, 2));
    return;
  }

  if (providers.length === 0) {
    console.log(`No AI providers configured for tenant '${tenant}'.`);
    return;
  }

  console.log(
    formatTable(
      ['PROVIDER', 'ENABLED', 'HAS API KEY', 'MODELS'],
      providers.map((p) => [
        p.provider,
        String(p.enabled),
        String(p.has_api_key),
        String(p.model_count),
      ])
    )
  );
}

export interface AiProviderTestOptions {
  tenant?: string;
}

export async function aiProviderTest(
  provider: string,
  options: AiProviderTestOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  const tenant = options.tenant || 'default';
  const result = await apiCall<{ success: boolean; message?: string; error?: string }>(
    `/api/tenants/${encodeURIComponent(tenant)}/ai/providers/${encodeURIComponent(provider)}/test`,
    { method: 'POST', fetchImpl }
  );

  if (!result.ok || !result.data) {
    throw new Error(`Provider test failed for '${provider}': ${result.errorMessage}`);
  }
  if (!result.data.success) {
    throw new Error(
      `Provider '${provider}' connection test FAILED: ${result.data.error || result.data.message || 'unknown error'}`
    );
  }
  console.log(
    `Provider '${provider}' connection OK${result.data.message ? `: ${result.data.message}` : '.'}`
  );
}
