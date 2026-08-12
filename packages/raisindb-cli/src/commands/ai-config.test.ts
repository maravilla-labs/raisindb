import { describe, it, expect } from 'vitest';
import {
  AIConfigResponse,
  KNOWN_PROVIDERS,
  SLUG_PATTERN,
  mergeProviderConfig,
  parseModelSpec,
  resolveApiKey,
  validateProviderSlug,
} from './ai-config.js';
import { redactSecret, formatTable } from './admin-util.js';

describe('parseModelSpec', () => {
  it('parses a bare model id with sane defaults', () => {
    const model = parseModelSpec('llama-3.3-70b-versatile', 0);
    expect(model).toEqual({
      model_id: 'llama-3.3-70b-versatile',
      display_name: 'llama-3.3-70b-versatile',
      use_cases: ['chat', 'agent'],
      default_temperature: 0.3,
      default_max_tokens: 1024,
      is_default: true,
    });
  });

  it('parses model_id:display_name', () => {
    const model = parseModelSpec('gpt-4o:GPT-4 Omni', 1);
    expect(model.model_id).toBe('gpt-4o');
    expect(model.display_name).toBe('GPT-4 Omni');
    expect(model.is_default).toBe(false);
  });

  it('only the first model is the default', () => {
    expect(parseModelSpec('a', 0).is_default).toBe(true);
    expect(parseModelSpec('b', 1).is_default).toBe(false);
    expect(parseModelSpec('c', 2).is_default).toBe(false);
  });

  it('rejects empty model ids', () => {
    expect(() => parseModelSpec('', 0)).toThrow(/Invalid --model spec/);
    expect(() => parseModelSpec(':DisplayOnly', 0)).toThrow(/Invalid --model spec/);
  });
});

describe('resolveApiKey', () => {
  it('returns null when no key flag is given (key unchanged)', async () => {
    expect(await resolveApiKey({})).toBeNull();
  });

  it('resolves --api-key', async () => {
    const result = await resolveApiKey({ apiKey: 'sk-test-123' });
    expect(result).toEqual({ key: 'sk-test-123', source: 'flag' });
  });

  it('resolves --api-key-env from the provided env', async () => {
    const result = await resolveApiKey(
      { apiKeyEnv: 'MY_KEY' },
      { env: { MY_KEY: '  gsk-from-env  ' } }
    );
    expect(result).toEqual({ key: 'gsk-from-env', source: 'env:MY_KEY' });
  });

  it('errors when the env var is missing, without leaking values', async () => {
    await expect(resolveApiKey({ apiKeyEnv: 'NOPE' }, { env: {} })).rejects.toThrow(
      /NOPE is not set or empty/
    );
  });

  it('resolves --api-key-stdin via the injected reader', async () => {
    const result = await resolveApiKey(
      { apiKeyStdin: true },
      { readStdinImpl: async () => 'stdin-key\n' }
    );
    expect(result).toEqual({ key: 'stdin-key', source: 'stdin' });
  });

  it('rejects empty stdin', async () => {
    await expect(
      resolveApiKey({ apiKeyStdin: true }, { readStdinImpl: async () => '\n' })
    ).rejects.toThrow(/No API key received on stdin/);
  });

  it('rejects multiple key sources', async () => {
    await expect(resolveApiKey({ apiKey: 'a', apiKeyStdin: true })).rejects.toThrow(
      /mutually exclusive/
    );
    await expect(resolveApiKey({ apiKey: 'a', apiKeyEnv: 'B' })).rejects.toThrow(
      /mutually exclusive/
    );
  });

  it('never includes the key value in error messages', async () => {
    const secret = 'super-secret-key-value';
    try {
      await resolveApiKey({ apiKey: secret, apiKeyStdin: true });
      expect.unreachable('should have thrown');
    } catch (e) {
      expect((e as Error).message).not.toContain(secret);
    }
  });
});

describe('redactSecret', () => {
  it('never exposes any part of the secret', () => {
    const secret = 'gsk_abcdef1234567890';
    const redacted = redactSecret(secret);
    expect(redacted).not.toContain('gsk');
    expect(redacted).not.toContain('abcdef');
    expect(redacted).toContain('redacted');
  });
});

describe('validateProviderSlug', () => {
  it('accepts the slugs the contract allows', () => {
    for (const slug of ['marvel', 'openai', 'my-vllm', 'gw2', 'a', 'a'.repeat(39)]) {
      expect(() => validateProviderSlug(slug)).not.toThrow();
    }
  });

  it('accepts every kind name, including the one the pattern rejects', () => {
    for (const kind of KNOWN_PROVIDERS) {
      expect(() => validateProviderSlug(kind)).not.toThrow();
    }
    // Why the exception exists: azure_openai is the legacy default slug of the
    // AzureOpenAI kind, and its underscore is outside the pattern.
    expect(SLUG_PATTERN.test('azure_openai')).toBe(false);
    expect(() => validateProviderSlug('azure_openai')).not.toThrow();
  });

  it('grandfathers exact kind names only, not anything else with an underscore', () => {
    expect(() => validateProviderSlug('azure_openai_2')).toThrow(/Invalid provider slug/);
    expect(() => validateProviderSlug('some_new_bad_slug')).toThrow(/Invalid provider slug/);
  });

  it("rejects ':' because it separates the slug from the model in model ids", () => {
    expect(() => validateProviderSlug('marvel:gpt-4o')).toThrow(/model ids/);
    expect(() => validateProviderSlug(':marvel')).toThrow(/model ids/);
  });

  it('rejects uppercase, so a slug has exactly one spelling', () => {
    expect(() => validateProviderSlug('Marvel')).toThrow(/lowercase/);
    expect(() => validateProviderSlug('MARVEL')).toThrow(/lowercase/);
  });

  it('rejects empty, leading dashes, spaces and over-long slugs', () => {
    expect(() => validateProviderSlug('')).toThrow(/Invalid provider slug/);
    expect(() => validateProviderSlug('-marvel')).toThrow(/Invalid provider slug/);
    expect(() => validateProviderSlug('my provider')).toThrow(/Invalid provider slug/);
    expect(() => validateProviderSlug('a'.repeat(40))).toThrow(/Invalid provider slug/);
  });
});

describe('mergeProviderConfig (read-modify-write, keyed by slug)', () => {
  const current: AIConfigResponse = {
    tenant_id: 'default',
    providers: [
      {
        slug: 'openai',
        provider: 'openai',
        has_api_key: true,
        api_endpoint: null,
        enabled: true,
        models: [
          {
            model_id: 'gpt-4o',
            display_name: 'GPT-4o',
            use_cases: ['chat'],
            default_temperature: 0.7,
            default_max_tokens: 2048,
            is_default: true,
          },
        ],
      },
      {
        slug: 'groq',
        provider: 'groq',
        has_api_key: true,
        api_endpoint: 'https://api.groq.com',
        enabled: false,
        models: [],
      },
    ],
    embedding_settings: { some: 'setting' },
  };

  it('preserves other providers without api_key_plain (server keeps their stored keys)', () => {
    const body = mergeProviderConfig(current, 'groq', { apiKey: 'new-key', enabled: true });
    const openai = body.providers.find((p) => p.slug === 'openai');
    expect(openai).toBeDefined();
    expect(openai).not.toHaveProperty('api_key_plain');
    expect(openai!.enabled).toBe(true);
    expect(openai!.models).toHaveLength(1);
  });

  it('updates only the named provider and sends api_key_plain only for it', () => {
    const body = mergeProviderConfig(current, 'groq', { apiKey: 'new-key', enabled: true });
    const groq = body.providers.find((p) => p.slug === 'groq');
    expect(groq!.api_key_plain).toBe('new-key');
    expect(groq!.enabled).toBe(true);
    expect(groq!.api_endpoint).toBe('https://api.groq.com'); // kept from existing
    expect(body.providers).toHaveLength(2);
  });

  it('omits api_key_plain for the target when no key was given (stored key preserved)', () => {
    const body = mergeProviderConfig(current, 'groq', { enabled: true });
    const groq = body.providers.find((p) => p.slug === 'groq');
    expect(groq).not.toHaveProperty('api_key_plain');
  });

  it('keeps existing models and enabled state when not overridden', () => {
    const body = mergeProviderConfig(current, 'openai', { apiKey: 'rotated' });
    const openai = body.providers.find((p) => p.slug === 'openai');
    expect(openai!.enabled).toBe(true);
    expect(openai!.models).toHaveLength(1);
    expect(openai!.models[0].model_id).toBe('gpt-4o');
    expect(openai!.api_key_plain).toBe('rotated');
  });

  it('inserts a new provider as enabled by default', () => {
    const body = mergeProviderConfig(current, 'anthropic', { kind: 'anthropic', apiKey: 'k' });
    const anthropic = body.providers.find((p) => p.slug === 'anthropic');
    expect(anthropic).toBeDefined();
    expect(anthropic!.enabled).toBe(true);
    expect(anthropic!.models).toEqual([]);
    expect(body.providers).toHaveLength(3);
  });

  it('requires a kind for a new slug but not for an existing one', () => {
    expect(() => mergeProviderConfig(current, 'brand-new', { apiKey: 'k' })).toThrow(/--kind/);
    expect(() => mergeProviderConfig(current, 'groq', { apiKey: 'k' })).not.toThrow();
  });

  it('keeps the stored kind of an existing slug (a slug never changes what it points at)', () => {
    const body = mergeProviderConfig(current, 'groq', { kind: 'openai' });
    expect(body.providers.find((p) => p.slug === 'groq')!.provider).toBe('groq');
  });

  it('keeps two providers of the same kind distinct', () => {
    const twoGateways: AIConfigResponse = {
      tenant_id: 'default',
      providers: [
        {
          slug: 'marvel',
          provider: 'custom',
          api_endpoint: 'https://marvel.maravilla.cloud/v1',
          display_name: 'Maravilla',
          enabled: true,
          models: [],
        },
        {
          slug: 'my-vllm',
          provider: 'custom',
          api_endpoint: 'https://vllm.internal/v1',
          enabled: true,
          models: [],
        },
      ],
    };

    const body = mergeProviderConfig(twoGateways, 'my-vllm', { apiKey: 'k2', enabled: false });
    expect(body.providers).toHaveLength(2);

    const marvel = body.providers.find((p) => p.slug === 'marvel')!;
    expect(marvel).not.toHaveProperty('api_key_plain');
    expect(marvel.enabled).toBe(true);
    expect(marvel.api_endpoint).toBe('https://marvel.maravilla.cloud/v1');
    expect(marvel.display_name).toBe('Maravilla');

    const vllm = body.providers.find((p) => p.slug === 'my-vllm')!;
    expect(vllm.api_key_plain).toBe('k2');
    expect(vllm.enabled).toBe(false);
  });

  it('carries display_name and icon_url through for the target and the untouched entries', () => {
    const body = mergeProviderConfig(current, 'groq', {
      displayName: 'Groq Cloud',
      iconUrl: 'https://example.test/groq.png',
    });
    const groq = body.providers.find((p) => p.slug === 'groq')!;
    expect(groq.display_name).toBe('Groq Cloud');
    expect(groq.icon_url).toBe('https://example.test/groq.png');

    // The same fields must survive on entries this call does not target,
    // because PUT replaces the document wholesale.
    const stored: AIConfigResponse = {
      tenant_id: 'default',
      providers: [
        {
          slug: 'marvel',
          provider: 'custom',
          display_name: 'Maravilla',
          icon_url: 'https://www.maravilla.cloud/maravilla-logo.png',
          enabled: true,
          models: [],
        },
      ],
    };
    const untouched = mergeProviderConfig(stored, 'openai', { kind: 'openai' }).providers.find(
      (p) => p.slug === 'marvel'
    )!;
    expect(untouched.display_name).toBe('Maravilla');
    expect(untouched.icon_url).toBe('https://www.maravilla.cloud/maravilla-logo.png');
  });

  it('treats a stored entry with no slug as slug == kind (pre-slug configs)', () => {
    const legacy: AIConfigResponse = {
      tenant_id: 'default',
      providers: [{ provider: 'openai', enabled: true, models: [] }],
    };
    const body = mergeProviderConfig(legacy, 'openai', { apiKey: 'rotated' });
    expect(body.providers).toHaveLength(1);
    expect(body.providers[0]).toMatchObject({ slug: 'openai', provider: 'openai' });
  });

  it('passes embedding_settings through (PUT replaces the whole document)', () => {
    const body = mergeProviderConfig(current, 'groq', {});
    expect(body.embedding_settings).toEqual({ some: 'setting' });
  });

  it('omits embedding_settings when the tenant has none', () => {
    const noEmbeddings: AIConfigResponse = { tenant_id: 't', providers: [] };
    const body = mergeProviderConfig(noEmbeddings, 'groq', { kind: 'groq', apiKey: 'k' });
    expect(body).not.toHaveProperty('embedding_settings');
  });

  it('replaces models when --model specs are given', () => {
    const models = [parseModelSpec('llama-3.3-70b-versatile', 0)];
    const body = mergeProviderConfig(current, 'groq', { models });
    const groq = body.providers.find((p) => p.slug === 'groq');
    expect(groq!.models).toHaveLength(1);
    expect(groq!.models[0].model_id).toBe('llama-3.3-70b-versatile');
    expect(groq!.models[0].is_default).toBe(true);
  });
});

describe('formatTable', () => {
  it('produces plain aligned text without ANSI codes', () => {
    const table = formatTable(
      ['PROVIDER', 'ENABLED'],
      [
        ['groq', 'true'],
        ['openai', 'false'],
      ]
    );
    expect(table).toContain('PROVIDER');
    expect(table).toContain('groq');
    // eslint-disable-next-line no-control-regex
    expect(table).not.toMatch(/\u001b/);
  });
});
