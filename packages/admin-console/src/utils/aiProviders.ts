// SPDX-License-Identifier: BSL-1.1

/**
 * Tenant AI providers, addressed by SLUG.
 *
 * A tenant's AI configuration is a list of entries keyed by a per-tenant `slug`,
 * not by the provider KIND. Two entries can share a kind — a provisioned
 * `marvel` gateway and a hand-rolled `custom` one both speak the OpenAI wire
 * protocol — so anything that keys state by kind silently collapses them and
 * shows one entry's endpoint, models and API key under the other's name.
 *
 * The slug is also the model-id prefix (`<slug>:<model>`), the value of an
 * agent node's `provider` property, and the value of `ai_provider_ref`. Nothing
 * holds a foreign key to it, so a wrong slug fails at call time rather than at
 * save time — which is why the console's job is to offer the tenant's real
 * slugs and never to invent one.
 *
 * PUT /ai/config is a MERGE keyed by slug: an entry in the payload is written
 * whole, an entry only in storage is left alone. Both halves matter here — see
 * `buildProviderRequests`.
 */

import type {
  AIModelConfig,
  AIProvider,
  ProviderConfigRequest,
  ProviderConfigResponse,
} from '../api/ai'

/** Every provider KIND the server knows, in the order they are offered. */
export const PROVIDER_KINDS: AIProvider[] = [
  'openai',
  'anthropic',
  'google',
  'azure_openai',
  'ollama',
  'groq',
  'openrouter',
  'bedrock',
  'local',
  'custom',
]

/** Human-readable name for a provider KIND (not for an entry — see `providerLabel`). */
const KIND_LABELS: Record<AIProvider, string> = {
  openai: 'OpenAI',
  anthropic: 'Anthropic',
  google: 'Google Gemini',
  azure_openai: 'Azure OpenAI',
  ollama: 'Ollama',
  groq: 'Groq',
  openrouter: 'OpenRouter',
  bedrock: 'AWS Bedrock',
  local: 'Local (Candle)',
  custom: 'Custom (OpenAI-compatible)',
}

export function kindLabel(kind: AIProvider): string {
  return KIND_LABELS[kind] ?? kind
}

/**
 * What to call one configured entry.
 *
 * The slug is the identity, so it is always shown; `display_name` is the label a
 * provisioned gateway ships and is what tells two `custom`-kind entries apart.
 */
export function providerLabel(entry: {
  slug: string
  display_name?: string
}): string {
  const name = entry.display_name?.trim()
  return name ? `${name} (${entry.slug})` : entry.slug
}

/**
 * Client-side mirror of the server's `validate_slug`.
 *
 * Kept in sync with `crates/raisin-ai/src/config/provider.rs`: 1-39 chars,
 * lowercase alphanumerics and dashes, no leading dash.
 */
export const PROVIDER_SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,38}$/

/**
 * Why a slug cannot be used, or `null` when it can.
 *
 * `existingSlugs` are the slugs already in play; a slug is an address, so a
 * duplicate would make `<slug>:<model>` ambiguous and the server rejects it.
 */
export function validateProviderSlug(
  slug: string,
  existingSlugs: readonly string[] = []
): string | null {
  if (!slug) return 'Slug is required'
  if (existingSlugs.includes(slug)) return `Slug '${slug}' is already in use`
  // A slug that names a KIND is always allowed, matching the server's first
  // branch. Without it `azure_openai` — the legacy default slug every pre-slug
  // Azure entry carries — would be unusable, because of the underscore.
  if ((PROVIDER_KINDS as string[]).includes(slug)) return null
  if (!PROVIDER_SLUG_PATTERN.test(slug)) {
    return 'Slug must be 1-39 characters of lowercase letters, digits or dashes, and start with a letter or digit'
  }
  return null
}

/**
 * Provider KINDS whose default model set can produce embeddings.
 *
 * Only a fallback: an entry that actually lists an embedding-capable model is
 * offered whatever its kind, which is how a `custom` gateway publishing
 * `maravilla/embed` shows up in the embedding picker.
 */
export const EMBEDDING_CAPABLE_KINDS: AIProvider[] = ['openai', 'ollama', 'local']

export function isEmbeddingCapable(entry: {
  provider: AIProvider
  models?: AIModelConfig[]
}): boolean {
  if (entry.models?.some((m) => m.use_cases.includes('embedding'))) return true
  return EMBEDDING_CAPABLE_KINDS.includes(entry.provider)
}

/** Kinds the server refuses to use without a credential.
 *  Mirrors `AIProvider::requires_api_key()` in
 *  `crates/raisin-ai/src/config/provider.rs`. */
const KINDS_REQUIRING_KEY: ReadonlySet<string> = new Set([
  'openai',
  'anthropic',
  'google',
  'azure_openai',
  'groq',
  'openrouter',
  'bedrock',
])

/** Entries a picker should offer: configured and usable right now. */
export function usableProviders(
  providers: readonly ProviderConfigResponse[]
): ProviderConfigResponse[] {
  // `local`, `ollama` and `custom` are all legitimately keyless — `local` runs in-process,
  // and the other two may point at an endpoint on a private network that authenticates by
  // reachability. Listing only `local` and `ollama` here hid a working self-hosted
  // `custom` endpoint from every picker while the server was perfectly willing to route to
  // it, which reads as "the product is broken" rather than "this list is over-filtered".
  return providers.filter(
    (p) => p.enabled && (p.has_api_key || !KINDS_REQUIRING_KEY.has(p.provider))
  )
}

/** Look up one entry by slug. Never by kind — that is the bug this module exists for. */
export function findProviderBySlug(
  providers: readonly ProviderConfigResponse[],
  slug: string | undefined
): ProviderConfigResponse | undefined {
  if (!slug) return undefined
  return providers.find((p) => p.slug === slug)
}

// ============================================================================
// Editable drafts
// ============================================================================

/**
 * One provider entry as the settings form holds it.
 *
 * Strings rather than optionals because these are bound to inputs; the
 * conversion back to "omitted" happens in `buildProviderRequests`.
 */
export interface ProviderDraft {
  /** Immutable identity. Editable only while `isNew`. */
  slug: string
  /** Wire protocol. Chosen at creation; the server rejects changing it. */
  kind: AIProvider
  displayName: string
  iconUrl: string
  apiEndpoint: string
  enabled: boolean
  /** A key typed in this session. Empty means "keep whatever is stored". */
  apiKey: string
  /** True until the entry has been saved once. */
  isNew: boolean
  /** Per-entry UI state, not part of the payload. */
  testing: boolean
  refreshing: boolean
  testResult?: { success: boolean; error?: string }
}

/**
 * One draft per stored entry, in server order.
 *
 * The list is the state — keying by kind is what let a tenant's `marvel`
 * gateway populate the `custom` row and then be saved back under it.
 */
export function draftsFromConfig(
  providers: readonly ProviderConfigResponse[]
): ProviderDraft[] {
  return providers.map((p) => ({
    slug: p.slug,
    kind: p.provider,
    displayName: p.display_name ?? '',
    iconUrl: p.icon_url ?? '',
    apiEndpoint: p.api_endpoint ?? '',
    enabled: p.enabled,
    apiKey: '',
    isNew: false,
    testing: false,
    refreshing: false,
  }))
}

/**
 * Has this draft been touched relative to what the server returned?
 *
 * A key typed in this session always counts as a change; `has_api_key` cannot
 * tell us whether the typed key differs from the stored one.
 */
export function isDraftDirty(
  draft: ProviderDraft,
  original: ProviderConfigResponse | undefined
): boolean {
  if (draft.isNew || !original) return true
  return (
    draft.apiKey !== '' ||
    draft.enabled !== original.enabled ||
    draft.kind !== original.provider ||
    draft.displayName.trim() !== (original.display_name ?? '') ||
    draft.iconUrl.trim() !== (original.icon_url ?? '') ||
    draft.apiEndpoint.trim() !== (original.api_endpoint ?? '')
  )
}

/**
 * The `providers` array to PUT: only the entries the operator actually touched.
 *
 * Two rules, both learned the hard way:
 *
 * 1. Send only dirty entries. PUT is a merge, so an omitted slug is left
 *    untouched — the safe direction. The previous console sent a fixed row per
 *    KIND on every save, which under the old replace semantics deleted the
 *    tenant's `marvel` gateway and its encrypted key.
 * 2. Send whole entries. `models` is written whole, so it is carried over from
 *    the stored entry — otherwise saving an endpoint edit wipes a discovered
 *    model list.
 * 3. Say what you mean about the descriptive fields. On the server an omitted
 *    `display_name`/`icon_url`/`api_endpoint` KEEPS the stored value and an
 *    explicit `null` clears it, so a field the operator emptied is sent as
 *    `null` — omitting it would silently put the old value back in a form that
 *    now shows it blank. Never `""`: that stores a `Some("")` which renders as
 *    a nameless provider.
 */
export function buildProviderRequests(
  drafts: readonly ProviderDraft[],
  originals: readonly ProviderConfigResponse[]
): ProviderConfigRequest[] {
  const bySlug = new Map(originals.map((p) => [p.slug, p]))

  return drafts
    .filter((draft) => isDraftDirty(draft, bySlug.get(draft.slug)))
    .map((draft) => {
      const original = bySlug.get(draft.slug)
      const request: ProviderConfigRequest = {
        // Never omitted. Absent means "the kind's serde name" on the server,
        // which for a second same-kind entry addresses the wrong row.
        slug: draft.slug,
        provider: draft.kind,
        enabled: draft.enabled,
        models: original?.models ?? [],
      }
      // `|| null` rather than an omission: the form holds the operator's whole
      // intent for these fields, and a blank one means "remove it".
      request.display_name = draft.displayName.trim() || null
      request.icon_url = draft.iconUrl.trim() || null
      request.api_endpoint = draft.apiEndpoint.trim() || null
      // Omitted unless typed: that is how the server knows to carry the stored
      // key forward, and the console never holds a plaintext key it did not see.
      if (draft.apiKey) request.api_key_plain = draft.apiKey
      return request
    })
}
