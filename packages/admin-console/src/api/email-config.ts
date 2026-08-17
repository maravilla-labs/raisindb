// SPDX-License-Identifier: BSL-1.1

/**
 * The tenant's outbound email configuration.
 *
 * There is no dedicated HTTP endpoint for this: the config is a
 * `raisin:EmailConfig` NODE at `/config/email` in the `raisin:system` workspace,
 * so this module addresses it through the ordinary node API. That is the same
 * node `raisin.email.send` reads on the server (raisin-functions
 * `api/raisindb/email.rs`), which is what keeps the console and the send path
 * from drifting apart.
 *
 * The API credential is deliberately NOT here. The node carries only a
 * `credential_ref` (`secret://email/api_key`); the key itself lives in the
 * secret store, which has no route that returns a value. See the Secrets page.
 */

import { nodesApi, type Node } from './nodes'

/** Workspace holding per-tenant operator configuration. */
export const CONFIG_WORKSPACE = 'raisin:system'
/** Parent of the config node. */
const CONFIG_PARENT = '/config'
/** Path of the `raisin:EmailConfig` node. */
export const EMAIL_CONFIG_PATH = '/config/email'
/** Node name (the last path segment). */
const EMAIL_CONFIG_NAME = 'email'

export const EMAIL_NODE_TYPE = 'raisin:EmailConfig'

/** v1 ships HTTP providers only — the sandbox has no raw TCP, so no SMTP. */
export const EMAIL_PROVIDERS = ['resend', 'brevo'] as const
export type EmailProvider = (typeof EMAIL_PROVIDERS)[number]

export interface EmailConfig {
  /**
   * Master switch. Absent reads as NOT enabled on the server, so this is
   * modelled as a plain boolean and written explicitly.
   */
  enabled: boolean
  provider: EmailProvider
  /** Must be a domain verified with the tenant's own provider account. */
  from_address: string
  from_name?: string
  reply_to?: string
  /**
   * Absolute base URL of the tenant's front end. Magic links are built from
   * this, so a wrong value produces working links pointing at the wrong host.
   */
  base_url: string
  /** Where the API key lives in the secret store. */
  credential_ref: string
}

export const EMAIL_CONFIG_DEFAULTS: EmailConfig = {
  // Matches the nodetype: deny by default. A tenant that has provisioned the
  // node but not finished verifying its sending domain must not send.
  enabled: false,
  provider: 'resend',
  from_address: '',
  from_name: '',
  reply_to: '',
  base_url: '',
  credential_ref: 'secret://email/api_key',
}

/** The secret name a `secret://name` reference points at, for linking to it. */
export function secretNameOf(credentialRef: string): string | null {
  const trimmed = credentialRef.trim()
  if (!trimmed.startsWith('secret://')) return null
  const name = trimmed.slice('secret://'.length).split('@')[0]
  return name || null
}

function fromNode(node: Node): EmailConfig {
  const p = (node.properties ?? {}) as Record<string, unknown>
  const str = (k: keyof EmailConfig, fallback = '') =>
    typeof p[k] === 'string' ? (p[k] as string) : fallback
  return {
    // Anything other than an explicit `true` is not enabled — mirroring the
    // server, which checks `!= Some(true)` rather than truthiness.
    enabled: p.enabled === true,
    provider: EMAIL_PROVIDERS.includes(p.provider as EmailProvider)
      ? (p.provider as EmailProvider)
      : 'resend',
    from_address: str('from_address'),
    from_name: str('from_name'),
    reply_to: str('reply_to'),
    base_url: str('base_url'),
    credential_ref: str('credential_ref', EMAIL_CONFIG_DEFAULTS.credential_ref),
  }
}

/**
 * Only the fields the nodetype declares, with blanks omitted rather than
 * written as empty strings — an empty `reply_to` should be absent, not a header
 * with nothing in it.
 */
function toProperties(config: EmailConfig): Record<string, unknown> {
  const props: Record<string, unknown> = {
    enabled: config.enabled,
    provider: config.provider,
    from_address: config.from_address.trim(),
    base_url: config.base_url.trim(),
    credential_ref: config.credential_ref.trim(),
  }
  for (const key of ['from_name', 'reply_to'] as const) {
    const value = (config[key] ?? '').trim()
    if (value) props[key] = value
  }
  return props
}

export const emailConfigApi = {
  /**
   * The current config, or `null` when the node does not exist yet.
   *
   * A missing node is the normal starting state for every tenant, so it is not
   * an error here — the page renders defaults and creates on first save.
   */
  get: async (repo: string, branch: string): Promise<EmailConfig | null> => {
    try {
      const node = await nodesApi.getAtHead(repo, branch, CONFIG_WORKSPACE, EMAIL_CONFIG_PATH)
      return node ? fromNode(node) : null
    } catch (e) {
      const status = (e as { status?: number })?.status
      if (status === 404) return null
      throw e
    }
  },

  /** Create the node on first save, update it thereafter. */
  save: async (
    repo: string,
    branch: string,
    config: EmailConfig,
    exists: boolean
  ): Promise<EmailConfig> => {
    const properties = toProperties(config)
    const commit = { message: 'Update outbound email configuration' }
    const node = exists
      ? await nodesApi.update(repo, branch, CONFIG_WORKSPACE, EMAIL_CONFIG_PATH, {
          properties,
          commit,
        })
      : await nodesApi.create(repo, branch, CONFIG_WORKSPACE, CONFIG_PARENT, {
          name: EMAIL_CONFIG_NAME,
          node_type: EMAIL_NODE_TYPE,
          properties,
          commit: { message: 'Configure outbound email' },
        })
    return fromNode(node)
  },
}

/** A field-keyed map of problems, empty when the config is publishable. */
export function validate(config: EmailConfig): Partial<Record<keyof EmailConfig, string>> {
  const errors: Partial<Record<keyof EmailConfig, string>> = {}

  // Only enforced when enabling. A half-filled draft that stays disabled is a
  // legitimate state — it is exactly what `enabled: false` is for.
  if (config.enabled) {
    if (!config.from_address.trim()) {
      errors.from_address = 'Required before email can be enabled'
    } else if (!/^[^@\s]+@[^@\s.]+\.[^@\s]+$/.test(config.from_address.trim())) {
      errors.from_address = 'Must be a single email address'
    }

    const base = config.base_url.trim()
    if (!base) {
      errors.base_url = 'Required before email can be enabled — magic links are built from it'
    } else {
      let url: URL | null = null
      try {
        url = new URL(base)
      } catch {
        url = null
      }
      if (!url) {
        errors.base_url = 'Must be an absolute URL, e.g. https://app.example.com'
      } else if (url.protocol !== 'https:' && url.hostname !== 'localhost') {
        // A magic link is a bearer credential in a URL. Sending one over http
        // to anything but localhost puts it in plaintext on the wire.
        errors.base_url = 'Must be https (except http://localhost for development)'
      }
    }
  }

  if (!config.credential_ref.trim()) {
    errors.credential_ref = 'Required'
  } else if (!config.credential_ref.trim().startsWith('secret://')) {
    // A literal key here would be written into a node property in the clear.
    errors.credential_ref = 'Must be a secret:// reference, never a key'
  }

  if (config.reply_to && !/^[^@\s]+@[^@\s.]+\.[^@\s]+$/.test(config.reply_to.trim())) {
    errors.reply_to = 'Must be a single email address'
  }

  return errors
}
