// SPDX-License-Identifier: BSL-1.1

/**
 * Regression tests for the slug-keyed provider model.
 *
 * Each case reproduces a failure the kind-keyed console actually produced: a
 * provisioned `marvel` gateway sitting next to a hand-made `custom` one, which
 * the old code collapsed into a single row and then saved back under the wrong
 * identity.
 */

import { describe, expect, it } from 'vitest'
import type { ProviderConfigResponse } from '../api/ai'
import {
  buildProviderRequests,
  draftsFromConfig,
  findProviderBySlug,
  isDraftDirty,
  isEmbeddingCapable,
  validateProviderSlug,
} from './aiProviders'

/** A provisioned gateway: `custom` kind, self-named slug, name and logo. */
const marvel: ProviderConfigResponse = {
  slug: 'marvel',
  provider: 'custom',
  display_name: 'Marvel Gateway',
  icon_url: 'https://marvel.example/logo.svg',
  has_api_key: true,
  api_endpoint: 'https://gateway.marvel.example/v1',
  enabled: true,
  models: [
    {
      model_id: 'maravilla/smart',
      display_name: 'Maravilla Smart',
      use_cases: ['chat', 'agent'],
      default_temperature: 0.7,
      default_max_tokens: 4096,
    },
    {
      model_id: 'maravilla/embed',
      display_name: 'Maravilla Embed',
      use_cases: ['embedding'],
      default_temperature: 0,
      default_max_tokens: 8192,
      metadata: { embedding_length: 1536 },
    },
  ],
}

/** A second entry of the SAME kind, which the kind-keyed console erased. */
const houseGateway: ProviderConfigResponse = {
  slug: 'custom',
  provider: 'custom',
  has_api_key: true,
  api_endpoint: 'https://internal.example/v1',
  enabled: false,
  models: [],
}

const openai: ProviderConfigResponse = {
  slug: 'openai',
  provider: 'openai',
  has_api_key: true,
  enabled: true,
  models: [],
}

describe('draftsFromConfig', () => {
  it('keeps two providers of the same kind as two separate entries', () => {
    const drafts = draftsFromConfig([marvel, houseGateway])

    expect(drafts.map((d) => d.slug)).toEqual(['marvel', 'custom'])
  })

  it('never fills one entry from another entry of the same kind', () => {
    // The exact defect: a `custom` row populated from `marvel`, endpoint,
    // models and all, because both were keyed by kind.
    const drafts = draftsFromConfig([marvel, houseGateway])
    const custom = drafts.find((d) => d.slug === 'custom')!

    expect(custom.apiEndpoint).toBe('https://internal.example/v1')
    expect(custom.displayName).toBe('')
    expect(custom.enabled).toBe(false)
  })
})

describe('findProviderBySlug', () => {
  it('resolves a slug to its own entry, not to the first of its kind', () => {
    expect(findProviderBySlug([marvel, houseGateway], 'custom')).toBe(houseGateway)
  })
})

describe('isDraftDirty', () => {
  it('reports an untouched entry as unchanged', () => {
    const [draft] = draftsFromConfig([marvel])

    expect(isDraftDirty(draft, marvel)).toBe(false)
  })

  it('reports a typed API key as a change even though the stored key is invisible', () => {
    const [draft] = draftsFromConfig([marvel])

    expect(isDraftDirty({ ...draft, apiKey: 'sk-new' }, marvel)).toBe(true)
  })
})

describe('buildProviderRequests', () => {
  it('omits providers the operator did not touch', () => {
    // Under merge semantics an omitted slug is left alone; sending a row per
    // kind is what wrote marvel's endpoint back under slug `custom`.
    const drafts = draftsFromConfig([marvel, houseGateway, openai])
    const edited = drafts.map((d) => (d.slug === 'openai' ? { ...d, apiKey: 'sk-rotated' } : d))

    const payload = buildProviderRequests(edited, [marvel, houseGateway, openai])

    expect(payload.map((p) => p.slug)).toEqual(['openai'])
  })

  it('sends a slug on every entry it does send', () => {
    const drafts = draftsFromConfig([marvel]).map((d) => ({ ...d, enabled: false }))

    const [entry] = buildProviderRequests(drafts, [marvel])

    // An absent slug defaults to the kind's name server-side — `custom` here,
    // which is a different provider entirely.
    expect(entry.slug).toBe('marvel')
    expect(entry.provider).toBe('custom')
  })

  it('carries the stored model list through an edit to another field', () => {
    // The merge writes `models` whole, so an omitted list clears it and the
    // tenant loses every discovered model on an endpoint edit.
    const drafts = draftsFromConfig([marvel]).map((d) => ({
      ...d,
      apiEndpoint: 'https://gateway.marvel.example/v2',
    }))

    const [entry] = buildProviderRequests(drafts, [marvel])

    expect(entry.models?.map((m) => m.model_id)).toEqual([
      'maravilla/smart',
      'maravilla/embed',
    ])
  })

  it('preserves a display name and icon the operator never touched', () => {
    const drafts = draftsFromConfig([marvel]).map((d) => ({ ...d, enabled: false }))

    const [entry] = buildProviderRequests(drafts, [marvel])

    expect(entry.display_name).toBe('Marvel Gateway')
    expect(entry.icon_url).toBe('https://marvel.example/logo.svg')
  })

  it('clears a display name with null rather than an empty string', () => {
    // `''` would store a Some("") that renders as a nameless provider.
    const drafts = draftsFromConfig([marvel]).map((d) => ({ ...d, displayName: '   ' }))

    const [entry] = buildProviderRequests(drafts, [marvel])

    expect(entry.display_name).toBeNull()
  })

  it('omits the API key unless one was typed, so the stored key survives', () => {
    const drafts = draftsFromConfig([marvel]).map((d) => ({ ...d, enabled: false }))

    const [entry] = buildProviderRequests(drafts, [marvel])

    expect('api_key_plain' in entry).toBe(false)
  })

  it('sends a typed API key', () => {
    const drafts = draftsFromConfig([marvel]).map((d) => ({ ...d, apiKey: 'sk-new' }))

    const [entry] = buildProviderRequests(drafts, [marvel])

    expect(entry.api_key_plain).toBe('sk-new')
  })

  it('always sends a newly added provider', () => {
    const drafts = [
      ...draftsFromConfig([marvel]),
      {
        slug: 'eu-gateway',
        kind: 'custom' as const,
        displayName: '',
        iconUrl: '',
        apiEndpoint: 'https://eu.example/v1',
        enabled: true,
        apiKey: 'sk-eu',
        isNew: true,
        testing: false,
        refreshing: false,
      },
    ]

    const payload = buildProviderRequests(drafts, [marvel])

    expect(payload.map((p) => p.slug)).toEqual(['eu-gateway'])
  })
})

describe('validateProviderSlug', () => {
  it('accepts a lowercase dashed slug', () => {
    expect(validateProviderSlug('eu-gateway')).toBeNull()
  })

  it('accepts a slug that names a provider kind, including azure_openai', () => {
    // The legacy default slug of every pre-slug Azure entry; the underscore
    // fails the general pattern, and rejecting it would strand those entries.
    expect(validateProviderSlug('azure_openai')).toBeNull()
  })

  it('rejects uppercase, leading dashes and over-long slugs', () => {
    expect(validateProviderSlug('Marvel')).not.toBeNull()
    expect(validateProviderSlug('-marvel')).not.toBeNull()
    expect(validateProviderSlug('a'.repeat(40))).not.toBeNull()
  })

  it('rejects a slug that is already in use, because a slug is an address', () => {
    expect(validateProviderSlug('marvel', ['marvel'])).not.toBeNull()
  })
})

describe('isEmbeddingCapable', () => {
  it('offers a custom gateway that publishes an embedding model', () => {
    // marvel is `custom` kind — a kind allowlist would have hidden it, leaving
    // the tenant unable to point embeddings at the gateway it was provisioned with.
    expect(isEmbeddingCapable(marvel)).toBe(true)
  })

  it('does not offer a chat-only provider of a non-embedding kind', () => {
    expect(isEmbeddingCapable(houseGateway)).toBe(false)
  })
})
