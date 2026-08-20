import { describe, expect, it } from 'vitest'
import { normalizeMountPath, planBundle, type Integration, type MountBundle } from './integrations'

const integration: Integration = {
  name: 'maravilla-stripe',
  title: 'Stripe',
  path: '/integrations/maravilla-stripe',
  provider_type: 'stripe',
  adapter_function: '/adapters/stripe',
  enabled: true,
}

const bundle: MountBundle = {
  id: 'b',
  title: 'Stripe',
  default_root: '/stripe',
  mounts: [
    {
      key: 'sessions',
      title: 'Checkout sessions',
      subpath: 'sessions',
      default: true,
      mapping_function: '/mappers/stripe-default',
      sync_config: { resource: 'checkout_sessions', mode: 'hybrid' },
      write_config: { mode: 'submit', command_node_types: ['stripe:CheckoutSession'] },
    },
    { key: 'products', title: 'Products', subpath: 'products', sync_config: { resource: 'products' } },
  ],
}

describe('normalizeMountPath', () => {
  it('collapses slashes and never returns an empty path', () => {
    expect(normalizeMountPath('')).toBe('/')
    expect(normalizeMountPath('stripe/')).toBe('/stripe')
    expect(normalizeMountPath('//a//b/')).toBe('/a/b')
  })
})

describe('planBundle', () => {
  it('mints one mount per selected key under the chosen root', () => {
    const out = planBundle({
      integration,
      bundle,
      keys: ['sessions'],
      account_ref: 'acct_1',
      target_workspace: 'finance',
      root: 'stripe',
    })
    expect(out).toHaveLength(1)
    const m = out[0]
    expect(m.name).toBe('maravilla-stripe-sessions')
    expect(m.mount_path).toBe('/stripe/sessions')
    expect(m.integration_ref).toBe('/integrations/maravilla-stripe')
    expect(m.account_ref).toBe('acct_1')
    expect(m.target_branch).toBe('main')
    expect(m.mapping_function).toBe('/mappers/stripe-default')
    expect(m.write_config).toEqual({ mode: 'submit', command_node_types: ['stripe:CheckoutSession'] })
    expect(m.enabled).toBe(true)
  })

  it('copies the template config rather than aliasing it', () => {
    const [m] = planBundle({ integration, bundle, keys: ['sessions'], target_workspace: 'finance', root: '/stripe' })
    m.sync_config!.resource = 'x'
    expect(bundle.mounts[0].sync_config!.resource).toBe('checkout_sessions')
  })

  it('ignores keys the bundle does not know and keeps bundle order', () => {
    const out = planBundle({ integration, bundle, keys: ['products', 'sessions', 'nope'], target_workspace: 'finance', root: '/stripe' })
    expect(out.map((m) => m.name)).toEqual(['maravilla-stripe-sessions', 'maravilla-stripe-products'])
    expect(out[1].write_config).toEqual({})
  })
})
