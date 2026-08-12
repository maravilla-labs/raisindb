import { apiCall, formatTable, FetchLike } from './admin-util.js';
import {
  AIConfigResponse,
  ApiKeyDeps,
  ApiKeyFlags,
  KNOWN_PROVIDERS,
  entrySlug,
  mergeProviderConfig,
  parseModelSpec,
  resolveApiKey,
  validateProviderSlug,
} from './ai-config.js';

/**
 * `raisindb ai provider ...` - tenant AI provider configuration (gh-secret
 * style: the API key is never echoed, never logged).
 *
 * A provider is addressed by its per-tenant slug, so a tenant can run several
 * providers of the same kind; `--kind` names which kind a *new* slug is.
 *
 * Endpoints (raisin-transport-http, handlers/ai/config.rs):
 *   GET  /api/tenants/{tenant}/ai/config
 *   PUT  /api/tenants/{tenant}/ai/config        (MERGES by slug: an entry in
 *        the payload is created or updated, an entry only in storage is left
 *        alone, and a provider's stored key is preserved when api_key_plain is
 *        omitted. The CLI still sends the full list it read, which is a no-op
 *        under merge semantics and the correct body for a server still doing a
 *        document replace.)
 *   GET  /api/tenants/{tenant}/ai/providers
 *   POST /api/tenants/{tenant}/ai/providers/{slug}/test
 */

export interface AiProviderSetCliOptions extends ApiKeyFlags {
  kind?: string;
  endpoint?: string;
  displayName?: string;
  iconUrl?: string;
  enabled?: boolean;
  disabled?: boolean;
  model?: string[];
  tenant?: string;
}

export async function aiProviderSet(
  slug: string,
  options: AiProviderSetCliOptions,
  fetchImpl?: FetchLike,
  keyDeps?: ApiKeyDeps
): Promise<void> {
  // KNOWN_PROVIDERS now gates --kind, not the slug: the slug is user-chosen.
  if (options.kind !== undefined && !(KNOWN_PROVIDERS as readonly string[]).includes(options.kind)) {
    throw new Error(
      `Unknown provider kind '${options.kind}'. Known kinds: ${KNOWN_PROVIDERS.join(', ')}`
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

  const existing = (getResult.data.providers ?? []).find((p) => entrySlug(p) === slug);

  // Validate on CREATE only, exactly as the server does. Slugs are immutable,
  // so an update always carries a slug that is already stored - and a stored
  // slug may predate the pattern (`azure_openai` is the common one). Validating
  // updates too would leave such an entry with no way to be saved again.
  if (!existing) {
    validateProviderSlug(slug);
  }

  // A slug's kind is fixed at creation: there is no rename and no referential
  // integrity behind it, so re-pointing an existing slug at another kind would
  // silently break every model id and ai_provider_ref that names it - and would
  // hand the stored credential to a client speaking a different protocol.
  if (existing && options.kind !== undefined && options.kind !== existing.provider) {
    throw new Error(
      `Provider '${slug}' already exists with kind '${existing.provider}'; ` +
        `a slug's kind cannot be changed. Create a new slug instead.`
    );
  }

  const enabled = options.enabled ? true : options.disabled ? false : undefined;
  const body = mergeProviderConfig(getResult.data, slug, {
    kind: options.kind,
    apiKey: resolved?.key,
    endpoint: options.endpoint,
    displayName: options.displayName,
    iconUrl: options.iconUrl,
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
    `kind=${targetEntry.provider}`,
    `enabled=${targetEntry.enabled}`,
    `models=${targetEntry.models.length}`,
    resolved ? `api_key=updated (from ${resolved.source})` : 'api_key=unchanged',
  ];
  console.log(`Provider '${slug}' configured for tenant '${tenant}' (${details.join(', ')}).`);
}

/**
 * One row of GET /ai/providers - mirrors `ProviderSummary` in
 * handlers/ai/types.rs. The optional fields are `skip_serializing_if =
 * "Option::is_none"` on the Rust side, so they are genuinely absent (not null)
 * whenever the provider has not set them.
 */
interface ProviderSummary {
  slug: string;
  /** provider KIND (the wire key is `provider`) */
  provider: string;
  display_name?: string | null;
  icon_url?: string | null;
  api_endpoint?: string | null;
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

  // Slug first: it is the identity every other command takes as its argument.
  console.log(
    formatTable(
      ['SLUG', 'KIND', 'ENDPOINT', 'ENABLED', 'HAS API KEY', 'MODELS'],
      providers.map((p) => [
        p.slug || p.provider,
        p.provider,
        p.api_endpoint || '-',
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
  slug: string,
  options: AiProviderTestOptions = {},
  fetchImpl?: FetchLike
): Promise<void> {
  const tenant = options.tenant || 'default';
  // The path segment is the slug now; legacy entries keep working because
  // their shimmed slug is the old provider name.
  const result = await apiCall<{ success: boolean; message?: string; error?: string }>(
    `/api/tenants/${encodeURIComponent(tenant)}/ai/providers/${encodeURIComponent(slug)}/test`,
    { method: 'POST', fetchImpl }
  );

  if (!result.ok || !result.data) {
    throw new Error(`Provider test failed for '${slug}': ${result.errorMessage}`);
  }
  if (!result.data.success) {
    throw new Error(
      `Provider '${slug}' connection test FAILED: ${result.data.error || result.data.message || 'unknown error'}`
    );
  }
  console.log(
    `Provider '${slug}' connection OK${result.data.message ? `: ${result.data.message}` : '.'}`
  );
}
