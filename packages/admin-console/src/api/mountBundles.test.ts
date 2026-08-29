import { describe, expect, it } from 'vitest'
import {
  activePrompts,
  missingPromptKeys,
  normalizeMountPath,
  planBundle,
  promptIsActive,
  type Integration,
  type MountBundle,
} from './integrations'

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

// A v5 bundle: two workspaces and three prompts, i.e. the Microsoft 365 shape.
const ms365: MountBundle = {
  id: 'ms365-workplace',
  title: 'Microsoft 365',
  default_workspace: 'workplace',
  default_root: '/microsoft-365',
  prompts: [
    {
      key: 'principal',
      title: 'Mailbox',
      type: 'remote',
      browse: 'mailbox',
      applies_to: ['inbox', 'outbox'],
      target: 'sync_config.principal',
    },
    {
      key: 'drive_scope',
      title: 'Drive',
      type: 'select',
      options: ['me', 'site'],
      applies_to: ['drive'],
      target: 'sync_config.drive_scope',
    },
    {
      key: 'site_id',
      title: 'SharePoint site',
      type: 'remote',
      browse: 'site',
      required: true,
      required_when: { drive_scope: 'site' },
      applies_to: ['drive'],
      target: 'sync_config.site_id',
    },
  ],
  mounts: [
    {
      key: 'inbox',
      title: 'Inbox',
      subpath: 'mail/inbox',
      remote_root: 'inbox',
      sync_config: { resource: 'mail' },
    },
    { key: 'outbox', title: 'Outbox', subpath: 'mail/outbox', sync_config: { resource: 'mail' } },
    {
      key: 'drive',
      title: 'Files',
      subpath: 'drives/onedrive',
      target_workspace: 'assets',
      root_override: '/',
      sync_config: { resource: 'files' },
    },
  ],
}

const ms: Integration = {
  name: 'maravilla-microsoft-365',
  title: 'Microsoft 365',
  path: '/integrations/maravilla-microsoft-365',
  provider_type: 'ms-graph',
  adapter_function: '/adapters/ms-graph',
  enabled: true,
}

describe('planBundle: per-entry workspace', () => {
  it('lands an entry in its own workspace and root, leaving the others alone', () => {
    const out = planBundle({
      integration: ms,
      bundle: ms365,
      keys: ['inbox', 'drive'],
      target_workspace: 'workplace',
      root: '/microsoft-365',
    })
    const [inbox, drive] = out
    expect(inbox.target_workspace).toBe('workplace')
    expect(inbox.mount_path).toBe('/microsoft-365/mail/inbox')
    expect(drive.target_workspace).toBe('assets')
    expect(drive.mount_path).toBe('/drives/onedrive')
  })
})

describe('planBundle: prompts', () => {
  it('writes an answer only onto the entries that ask for it', () => {
    const out = planBundle({
      integration: ms,
      bundle: ms365,
      keys: ['inbox', 'drive'],
      target_workspace: 'workplace',
      root: '/microsoft-365',
      answers: { principal: 'sales@contoso.com' },
    })
    expect(out[0].sync_config!.principal).toBe('sales@contoso.com')
    expect(out[1].sync_config!.principal).toBeUndefined()
  })

  it('ignores a blank answer rather than writing an empty string', () => {
    // "" is what the console writes for a cleared field, and the adapter reads
    // it as "unset" only because configStr trims — a mount should not carry it.
    const [inbox] = planBundle({
      integration: ms,
      bundle: ms365,
      keys: ['inbox'],
      target_workspace: 'workplace',
      root: '/microsoft-365',
      answers: { principal: '   ' },
    })
    expect(inbox.sync_config!.principal).toBeUndefined()
  })

  it('drops an answer whose prompt is no longer active', () => {
    // The operator typed a site id, then switched the drive back to `me`. The
    // stale site_id must not ride along: driveBase() infers `site` from it.
    const [drive] = planBundle({
      integration: ms,
      bundle: ms365,
      keys: ['drive'],
      target_workspace: 'workplace',
      root: '/microsoft-365',
      answers: { drive_scope: 'me', site_id: 'contoso.sharepoint.com,abc' },
    })
    expect(drive.sync_config!.drive_scope).toBe('me')
    expect(drive.sync_config!.site_id).toBeUndefined()
  })

  it('applies a conditional answer once its condition holds', () => {
    const [drive] = planBundle({
      integration: ms,
      bundle: ms365,
      keys: ['drive'],
      target_workspace: 'workplace',
      root: '/microsoft-365',
      answers: { drive_scope: 'site', site_id: 'contoso.sharepoint.com,abc' },
    })
    expect(drive.sync_config!.site_id).toBe('contoso.sharepoint.com,abc')
  })

  it('throws on a target outside the closed set instead of dropping it', () => {
    const evil: MountBundle = {
      ...ms365,
      prompts: [
        {
          key: 'x',
          title: 'x',
          type: 'text',
          applies_to: ['inbox'],
          target: 'write_config.mode' as never,
        },
      ],
    }
    expect(() =>
      planBundle({
        integration: ms,
        bundle: evil,
        keys: ['inbox'],
        target_workspace: 'workplace',
        root: '/microsoft-365',
        answers: { x: 'mirror' },
      })
    ).toThrow(/unsupported target/)
  })

  it('plans a v4 bundle exactly as before', () => {
    const before = planBundle({
      integration,
      bundle,
      keys: ['sessions', 'products'],
      target_workspace: 'finance',
      root: '/stripe',
    })
    expect(before.map((m) => [m.name, m.mount_path, m.target_workspace])).toEqual([
      ['maravilla-stripe-sessions', '/stripe/sessions', 'finance'],
      ['maravilla-stripe-products', '/stripe/products', 'finance'],
    ])
  })
})

describe('prompt visibility', () => {
  it('hides a conditional prompt until its condition holds', () => {
    expect(promptIsActive(ms365.prompts![2], { drive_scope: 'me' })).toBe(false)
    expect(promptIsActive(ms365.prompts![2], { drive_scope: 'site' })).toBe(true)
  })

  it('asks only what the selected entries need', () => {
    expect(activePrompts(ms365, ['inbox'], {}).map((p) => p.key)).toEqual(['principal'])
    expect(activePrompts(ms365, ['drive'], { drive_scope: 'site' }).map((p) => p.key)).toEqual([
      'drive_scope',
      'site_id',
    ])
  })

  it('reports a required prompt as missing only while it is active', () => {
    expect(missingPromptKeys(ms365, ['drive'], { drive_scope: 'me' })).toEqual([])
    expect(missingPromptKeys(ms365, ['drive'], { drive_scope: 'site' })).toEqual(['site_id'])
    // Not selected, so not asked, so not blocking.
    expect(missingPromptKeys(ms365, ['inbox'], { drive_scope: 'site' })).toEqual([])
  })
})
