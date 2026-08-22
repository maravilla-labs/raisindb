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
 * A tenant configures ONE OR MORE providers and marks one default. The default
 * is what system mail uses — magic-link sign-in above all — and a function may
 * name another with `raisin.email.send({ provider: "..." })`.
 *
 * API keys and SMTP passwords are deliberately NOT here. Each provider carries
 * only a `credential_ref` (`secret://email/api_key`); the value lives in the
 * secret store, which has no route that returns one. See the Secrets page.
 */

import { nodesApi, type Node } from './nodes'
import { invokeFunction } from './functions'

/** Workspace holding per-tenant operator configuration. */
export const CONFIG_WORKSPACE = 'raisin:system'
/** Parent of the config node. */
const CONFIG_PARENT = '/config'
/** Path of the `raisin:EmailConfig` node. */
export const EMAIL_CONFIG_PATH = '/config/email'
/** Node name (the last path segment). */
const EMAIL_CONFIG_NAME = 'email'

export const EMAIL_NODE_TYPE = 'raisin:EmailConfig'

/**
 * The provider APIs a sender can talk to. `smtp` is served natively by the
 * server (never by the function sandbox), so a relay is a first-class choice
 * rather than a workaround.
 */
export const EMAIL_PROVIDERS = ['resend', 'brevo', 'smtp'] as const
export type EmailProviderKind = (typeof EMAIL_PROVIDERS)[number]

/** How an SMTP session is secured. */
export const SMTP_SECURITY = ['starttls', 'tls', 'none'] as const
export type SmtpSecurity = (typeof SMTP_SECURITY)[number]

export interface SmtpSettings {
  host: string
  port: number
  username: string
  security: SmtpSecurity
}

/** One configured sender. */
export interface EmailProviderConfig {
  /** Unique slug a function names. Not the provider API. */
  name: string
  provider: EmailProviderKind
  /** Must be verified with THIS entry's account. */
  from_address: string
  from_name?: string
  reply_to?: string
  /** Where this entry's API key or SMTP password lives in the secret store. */
  credential_ref: string
  /** Optional provider API base override. Ignored by SMTP. */
  api_base?: string
  /** Connection settings, required when `provider` is `smtp`. */
  smtp?: SmtpSettings
  /** A disabled entry cannot be selected, by name or as the default. */
  enabled: boolean
}

export interface EmailConfig {
  /**
   * Master switch for the tenant. Absent reads as NOT enabled on the server, so
   * this is modelled as a plain boolean and written explicitly.
   */
  enabled: boolean
  /**
   * Absolute base URL of the tenant's front end. Magic links are built from
   * this, so a wrong value produces working links pointing at the wrong host.
   */
  base_url: string
  providers: EmailProviderConfig[]
  /** Name of the provider system mail goes through. */
  default_provider: string
}

export const SMTP_DEFAULTS: SmtpSettings = {
  host: '',
  port: 587,
  username: '',
  security: 'starttls',
}

export const EMAIL_CONFIG_DEFAULTS: EmailConfig = {
  // Matches the nodetype: deny by default. A tenant that has provisioned the
  // node but not finished verifying its sending domain must not send.
  enabled: false,
  base_url: '',
  providers: [],
  default_provider: '',
}

/** A blank provider entry, for the "Add provider" button. */
export function newProvider(kind: EmailProviderKind = 'resend'): EmailProviderConfig {
  return {
    name: '',
    provider: kind,
    from_address: '',
    from_name: '',
    reply_to: '',
    // Each entry carries its own reference, so rotating one account cannot
    // disturb another. The default name is only a suggestion.
    credential_ref: 'secret://email/api_key',
    enabled: true,
    ...(kind === 'smtp' ? { smtp: { ...SMTP_DEFAULTS } } : {}),
  }
}

/** The secret name a `secret://name` reference points at, for linking to it. */
export function secretNameOf(credentialRef: string): string | null {
  const trimmed = (credentialRef ?? '').trim()
  if (!trimmed.startsWith('secret://')) return null
  const name = trimmed.slice('secret://'.length).split('@')[0]
  return name || null
}

function str(source: Record<string, unknown>, key: string, fallback = ''): string {
  const value = source[key]
  return typeof value === 'string' ? value : fallback
}

function providerFrom(raw: unknown): EmailProviderConfig | null {
  if (!raw || typeof raw !== 'object') return null
  const p = raw as Record<string, unknown>
  const kind = EMAIL_PROVIDERS.includes(p.provider as EmailProviderKind)
    ? (p.provider as EmailProviderKind)
    : 'resend'
  const smtpRaw = (p.smtp ?? null) as Record<string, unknown> | null
  return {
    name: str(p, 'name'),
    provider: kind,
    from_address: str(p, 'from_address'),
    from_name: str(p, 'from_name'),
    reply_to: str(p, 'reply_to'),
    credential_ref: str(p, 'credential_ref', 'secret://email/api_key'),
    api_base: str(p, 'api_base'),
    // Absent `enabled` means enabled: an entry the tenant added on purpose is
    // live unless it says otherwise. This mirrors the server's serde default,
    // which is the opposite of the config-level switch and deliberately so.
    enabled: p.enabled !== false,
    smtp:
      kind === 'smtp' || smtpRaw
        ? {
            host: str(smtpRaw ?? {}, 'host'),
            port: typeof smtpRaw?.port === 'number' ? (smtpRaw.port as number) : 587,
            username: str(smtpRaw ?? {}, 'username'),
            security: SMTP_SECURITY.includes(smtpRaw?.security as SmtpSecurity)
              ? (smtpRaw?.security as SmtpSecurity)
              : 'starttls',
          }
        : undefined,
  }
}

function fromNode(node: Node): EmailConfig {
  const p = (node.properties ?? {}) as Record<string, unknown>
  const providers = Array.isArray(p.providers)
    ? (p.providers as unknown[]).map(providerFrom).filter((x): x is EmailProviderConfig => !!x)
    : []

  // A per-entry `default: true` is read back into the single `default_provider`
  // field, so the editor has ONE control for the default and cannot write two
  // spellings that disagree. The server resolves the same way round: an
  // explicit `default_provider` wins over the flags.
  let defaultName = str(p, 'default_provider')
  if (!defaultName && Array.isArray(p.providers)) {
    const flagged = (p.providers as Record<string, unknown>[]).find((e) => e?.default === true)
    if (flagged) defaultName = str(flagged, 'name')
  }

  return {
    // Anything other than an explicit `true` is not enabled — mirroring the
    // server, which checks `!= Some(true)` rather than truthiness.
    enabled: p.enabled === true,
    base_url: str(p, 'base_url'),
    providers,
    default_provider: defaultName,
  }
}

/**
 * Only the fields the nodetype declares, with blanks omitted rather than
 * written as empty strings — an empty `reply_to` should be absent, not a header
 * with nothing in it.
 */
function toProperties(config: EmailConfig): Record<string, unknown> {
  const providers = config.providers.map((p) => {
    const entry: Record<string, unknown> = {
      name: p.name.trim(),
      provider: p.provider,
      from_address: p.from_address.trim(),
      credential_ref: p.credential_ref.trim(),
      enabled: p.enabled,
      // Written alongside `default_provider` so the node reads correctly on its
      // own (in a diff, or to a reader of the raw properties). The server takes
      // `default_provider` as authoritative, so the two cannot disagree.
      default: !!config.default_provider && p.name.trim() === config.default_provider.trim(),
    }
    for (const key of ['from_name', 'reply_to', 'api_base'] as const) {
      const value = (p[key] ?? '').trim()
      if (value) entry[key] = value
    }
    if (p.provider === 'smtp' && p.smtp) {
      const smtp: Record<string, unknown> = {
        host: p.smtp.host.trim(),
        port: p.smtp.port,
        security: p.smtp.security,
      }
      const username = p.smtp.username.trim()
      if (username) smtp.username = username
      entry.smtp = smtp
    }
    return entry
  })

  return {
    enabled: config.enabled,
    base_url: config.base_url.trim(),
    providers,
    default_provider: config.default_provider.trim(),
  }
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

  /**
   * Send one real test message, to prove a provider actually delivers.
   *
   * Goes through the built-in `send-test-email` function, which calls
   * `raisin.email.send` like any other function would. That is deliberate: the
   * console can already see that the config is complete and that the referenced
   * secret EXISTS, but nothing reads a secret's VALUE back — so the first proof
   * that a key is the RIGHT key has always been a real send. A shortcut around
   * the ordinary path would prove the shortcut.
   *
   * `provider` empty means the tenant's default, which is what a magic link
   * uses.
   */
  sendTest: async (
    repo: string,
    to: string,
    provider: string
  ): Promise<{ ok: boolean; detail: string }> => {
    const input: Record<string, unknown> = { to: to.trim() }
    const named = provider.trim()
    if (named) input.provider = named

    const response = await invokeFunction(repo, 'send-test-email', { input, sync: true })
    if (response.error) {
      return { ok: false, detail: response.error }
    }
    const result = (response.result ?? {}) as Record<string, unknown>
    const messageId = typeof result.message_id === 'string' ? result.message_id : ''
    const sender = typeof result.sender === 'string' ? result.sender : named || 'default'
    return {
      ok: result.sent === true,
      detail: messageId ? `${sender} accepted it (${messageId})` : `sent through ${sender}`,
    }
  },
}

/** A field-keyed map of problems on one provider entry. */
export type ProviderErrors = Partial<Record<keyof EmailProviderConfig | 'smtp_host', string>>

/** Problems on the config as a whole, plus one map per provider (by index). */
export interface ValidationResult {
  base_url?: string
  default_provider?: string
  providers: Record<number, ProviderErrors>
}

const ADDRESS = /^[^@\s]+@[^@\s.]+\.[^@\s]+$/

/** Empty `providers` and no top-level key means the config is publishable. */
export function validate(config: EmailConfig): ValidationResult {
  const result: ValidationResult = { providers: {} }

  // Only enforced when enabling. A half-filled draft that stays disabled is a
  // legitimate state — it is exactly what `enabled: false` is for.
  if (config.enabled) {
    const base = config.base_url.trim()
    if (!base) {
      result.base_url = 'Required before email can be enabled — magic links are built from it'
    } else {
      let url: URL | null = null
      try {
        url = new URL(base)
      } catch {
        url = null
      }
      if (!url) {
        result.base_url = 'Must be an absolute URL, e.g. https://app.example.com'
      } else if (url.protocol !== 'https:' && url.hostname !== 'localhost') {
        // A magic link is a bearer credential in a URL. Sending one over http
        // to anything but localhost puts it in plaintext on the wire.
        result.base_url = 'Must be https (except http://localhost for development)'
      }
    }

    if (config.providers.length === 0) {
      result.default_provider = 'Add at least one provider before enabling email'
    } else {
      const live = config.providers.filter((p) => p.enabled)
      if (live.length === 0) {
        result.default_provider = 'Every provider is disabled, so nothing can send'
      } else if (live.length > 1 && !config.default_provider.trim()) {
        // The server refuses an ambiguous default rather than guessing, so the
        // console must not let one be saved as if it were fine.
        result.default_provider = 'Choose which provider system mail goes through'
      } else if (
        config.default_provider.trim() &&
        !live.some((p) => p.name.trim() === config.default_provider.trim())
      ) {
        result.default_provider = 'The default must be one of the enabled providers'
      }
    }
  }

  const seen = new Set<string>()
  config.providers.forEach((p, index) => {
    const errors: ProviderErrors = {}
    const name = p.name.trim()
    if (!name) {
      errors.name = 'Required — this is the name a function passes to send()'
    } else if (!/^[a-z0-9][a-z0-9_-]*$/i.test(name)) {
      errors.name = 'Letters, digits, dashes and underscores only'
    } else if (seen.has(name.toLowerCase())) {
      // Two entries with one name means a by-name send is a coin toss.
      errors.name = 'Another provider already uses this name'
    }
    if (name) seen.add(name.toLowerCase())

    if (!p.from_address.trim()) {
      errors.from_address = 'Required'
    } else if (!ADDRESS.test(p.from_address.trim())) {
      errors.from_address = 'Must be a single email address'
    }

    if (p.reply_to && !ADDRESS.test(p.reply_to.trim())) {
      errors.reply_to = 'Must be a single email address'
    }

    if (!p.credential_ref.trim()) {
      errors.credential_ref = 'Required'
    } else if (!p.credential_ref.trim().startsWith('secret://')) {
      // A literal key here would be written into a node property in the clear.
      errors.credential_ref = 'Must be a secret:// reference, never a key'
    }

    if (p.provider === 'smtp') {
      if (!p.smtp?.host.trim()) {
        errors.smtp_host = 'Required for an SMTP provider'
      }
      if (!p.smtp?.port || p.smtp.port < 1 || p.smtp.port > 65535) {
        errors.smtp_host = 'Port must be between 1 and 65535'
      }
    }

    if (Object.keys(errors).length > 0) result.providers[index] = errors
  })

  return result
}

/** True when nothing in a validation result blocks a save. */
export function isValid(result: ValidationResult): boolean {
  return (
    !result.base_url &&
    !result.default_provider &&
    Object.keys(result.providers).length === 0
  )
}
