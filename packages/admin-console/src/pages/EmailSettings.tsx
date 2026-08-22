// SPDX-License-Identifier: BSL-1.1

/**
 * Outbound (transactional) email configuration, per repository branch.
 *
 * The tenant brings their own provider accounts — Resend, Brevo, or any SMTP
 * relay — and verifies their own sending domains; mail goes out as them. There
 * is deliberately no platform fallback sender, so a tenant who has configured
 * nothing cannot send at all rather than sending as somebody else.
 *
 * Several providers may be configured and ONE is marked default. The default is
 * what system mail uses — magic-link sign-in above all — and a function may
 * name another with `raisin.email.send({ provider: "..." })`.
 *
 * Three states look fine and are not, so all three are called out in the UI:
 *
 *  - **Disabled.** The server treats an absent or non-`true` `enabled` as off.
 *    That is the correct default (a verified sending domain comes first), but a
 *    saved, complete-looking config that still sends nothing is confusing
 *    without being told why.
 *  - **No API key.** A provider names a `secret://` reference; the key itself
 *    lives in the secret store and nothing on this page can see it. A config
 *    can therefore be valid, enabled, and still fail on the first send.
 *  - **The WRONG API key.** Nothing reads a secret's value back, so a present
 *    secret is not a working one. That is what **Send test** is for: it is the
 *    only signal here that is not indirect.
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { AlertTriangle, KeyRound, Mail, Plus, Send, Star, Trash2 } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { useToast, ToastContainer } from '../components/Toast'
import {
  EMAIL_CONFIG_DEFAULTS,
  EMAIL_CONFIG_PATH,
  EMAIL_PROVIDERS,
  SMTP_DEFAULTS,
  SMTP_SECURITY,
  emailConfigApi,
  isValid,
  newProvider,
  secretNameOf,
  validate,
  type EmailConfig,
  type EmailProviderConfig,
  type EmailProviderKind,
  type ProviderErrors,
  type SmtpSecurity,
  type ValidationResult,
} from '../api/email-config'
import { secretsApi } from '../api/secrets'

/**
 * Where each provider's credential actually comes from. The Brevo line is the
 * one that earns its place: Brevo's REST API and its SMTP relay use DIFFERENT
 * credentials, both called "key", on two different settings pages — and an SMTP
 * key in the API field returns 401 on every send with no hint as to why.
 */
const PROVIDER_HELP: Record<EmailProviderKind, string> = {
  resend: 'API key from resend.com → API Keys. The sending domain must be verified there.',
  brevo:
    'v3 API KEY from brevo.com → Settings → API keys. NOT the SMTP key (Settings → SMTP & API → ' +
    'SMTP) — that is a different credential and returns 401 here. To use an SMTP key, add an ' +
    'smtp provider instead.',
  smtp:
    'Password for the SMTP account below — for Brevo’s relay, that is the SMTP key and the ' +
    'username is your account login.',
}

/** Ports each security mode conventionally uses, shown as a hint. */
const SECURITY_HELP: Record<SmtpSecurity, string> = {
  starttls: 'Connect in the clear, then upgrade. Submission default, port 587.',
  tls: 'TLS from the first byte. Port 465.',
  none: 'No encryption — the password crosses the wire in the clear. Trusted networks only.',
}

export default function EmailSettings() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const currentBranch = branch || 'main'

  const [config, setConfig] = useState<EmailConfig>(EMAIL_CONFIG_DEFAULTS)
  const [exists, setExists] = useState(false)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [errors, setErrors] = useState<ValidationResult>({ providers: {} })
  /**
   * Which referenced secrets exist. A name absent from the map means "not yet
   * determined", so a warning is never flashed before we know.
   */
  const [secretsPresent, setSecretsPresent] = useState<Record<string, boolean>>({})
  const [testTo, setTestTo] = useState('')
  const [testProvider, setTestProvider] = useState('')
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<{ ok: boolean; detail: string } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const load = useCallback(async () => {
    if (!repo) return
    setLoading(true)
    try {
      const loaded = await emailConfigApi.get(repo, currentBranch)
      setConfig(loaded ?? EMAIL_CONFIG_DEFAULTS)
      setExists(loaded !== null)
      setLoadError(null)
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [repo, currentBranch])

  useEffect(() => {
    void load()
  }, [load])

  // Every referenced secret, deduplicated — a name appearing on two providers
  // is one probe, not two.
  const referencedSecrets = useMemo(() => {
    const names = new Set<string>()
    for (const p of config.providers) {
      const name = secretNameOf(p.credential_ref)
      if (name) names.add(name)
    }
    return Array.from(names).sort()
  }, [config.providers])

  // Existence only — `list` is metadata, and there is no route that returns a
  // secret's value.
  useEffect(() => {
    if (!repo || referencedSecrets.length === 0) {
      setSecretsPresent({})
      return
    }
    let cancelled = false
    secretsApi
      .list(repo, currentBranch)
      .then((all) => {
        if (cancelled) return
        const live = new Set(all.filter((s) => !s.deleted).map((s) => s.name))
        setSecretsPresent(
          Object.fromEntries(referencedSecrets.map((name) => [name, live.has(name)]))
        )
      })
      .catch(() => {
        // A failed probe must not render as "missing" — that would send an
        // operator hunting for a secret that is present.
        if (!cancelled) setSecretsPresent({})
      })
    return () => {
      cancelled = true
    }
  }, [repo, currentBranch, referencedSecrets])

  const setField = <K extends keyof EmailConfig>(key: K, value: EmailConfig[K]) => {
    setConfig((c) => ({ ...c, [key]: value }))
    setErrors((e) => ({ ...e, [key]: undefined }))
  }

  const patchProvider = (index: number, patch: Partial<EmailProviderConfig>) => {
    setConfig((c) => {
      const providers = c.providers.map((p, i) => (i === index ? { ...p, ...patch } : p))
      // Renaming the default must carry the default with it, or saving would
      // write a `default_provider` that names nothing.
      const renamed = patch.name !== undefined && c.providers[index].name === c.default_provider
      return {
        ...c,
        providers,
        default_provider: renamed ? (patch.name as string) : c.default_provider,
      }
    })
  }

  const addProvider = () => {
    setConfig((c) => {
      const providers = [...c.providers, newProvider()]
      return { ...c, providers }
    })
  }

  const removeProvider = (index: number) => {
    setConfig((c) => {
      const removed = c.providers[index]
      const providers = c.providers.filter((_, i) => i !== index)
      return {
        ...c,
        providers,
        default_provider:
          removed.name === c.default_provider ? '' : c.default_provider,
      }
    })
    setErrors({ providers: {} })
  }

  const save = async () => {
    if (!repo) return
    const found = validate(config)
    setErrors(found)
    if (!isValid(found)) {
      showError('Fix the highlighted fields first')
      return
    }
    setSaving(true)
    try {
      const saved = await emailConfigApi.save(repo, currentBranch, config, exists)
      setConfig(saved)
      setExists(true)
      showSuccess(config.enabled ? 'Email configuration saved' : 'Saved — email is still disabled')
    } catch (e) {
      showError(e instanceof Error ? e.message : String(e))
    } finally {
      setSaving(false)
    }
  }

  const sendTest = async () => {
    if (!repo) return
    setTesting(true)
    setTestResult(null)
    try {
      const result = await emailConfigApi.sendTest(repo, testTo, testProvider)
      setTestResult(result)
      if (result.ok) showSuccess('Test message accepted by the provider')
      else showError('Test send failed')
    } catch (e) {
      setTestResult({ ok: false, detail: e instanceof Error ? e.message : String(e) })
      showError('Test send failed')
    } finally {
      setTesting(false)
    }
  }

  const enabledProviders = config.providers.filter((p) => p.enabled && p.name.trim())
  // Function invocation is main-only, so a test run from another branch would
  // exercise main's configuration and report on the wrong thing.
  const testableHere = currentBranch === 'main'

  if (loading) {
    return <div className="p-6 text-white/60">Loading…</div>
  }

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-center gap-3">
        <Mail className="w-6 h-6 text-primary-400" />
        <div>
          <h1 className="text-2xl font-semibold text-white">Outbound email</h1>
          <p className="text-sm text-white/60">
            Transactional mail — magic-link sign-in, notifications. Sent through your own
            provider accounts, from your own verified domains.
          </p>
        </div>
      </div>

      {loadError && (
        <GlassCard className="p-4 border border-red-400/40">
          <p className="text-sm text-red-300">Could not load configuration: {loadError}</p>
        </GlassCard>
      )}

      {!config.enabled && (
        <GlassCard className="p-4 border border-amber-400/40">
          <div className="flex gap-3">
            <AlertTriangle className="w-5 h-5 text-amber-400 flex-shrink-0" />
            <p className="text-sm text-amber-200">
              Email is <strong>disabled</strong>. Nothing is sent, and{' '}
              <code className="text-amber-100">raisin.email.send</code> fails with a
              configuration error — however many providers are configured below.
            </p>
          </div>
        </GlassCard>
      )}

      {config.enabled && config.providers.length === 0 && (
        <GlassCard className="p-4 border border-red-400/40">
          <div className="flex gap-3">
            <AlertTriangle className="w-5 h-5 text-red-400 flex-shrink-0" />
            <p className="text-sm text-red-200">
              Email is enabled but <strong>no provider is configured</strong>. There is no
              platform fallback sender, so every send fails. Add one below.
            </p>
          </div>
        </GlassCard>
      )}

      <GlassCard className="p-6 space-y-5">
        <label className="flex items-start gap-3 cursor-pointer">
          <input
            type="checkbox"
            className="mt-1"
            checked={config.enabled}
            onChange={(e) => setField('enabled', e.target.checked)}
          />
          <span>
            <span className="text-white font-medium">Enable outbound email</span>
            <span className="block text-xs text-white/50">
              Leave off until a sending domain is verified with your provider.
            </span>
          </span>
        </label>

        <Field label="Front-end base URL" error={errors.base_url} required={config.enabled}>
          <Input
            value={config.base_url}
            onChange={(v) => setField('base_url', v)}
            placeholder="https://app.example.com"
          />
          <Hint>
            Magic links are built from this. A wrong value produces links that work but point
            at the wrong host. It is never taken from a request header, on purpose.
          </Hint>
        </Field>

        <Field label="Default provider" error={errors.default_provider}>
          <select
            className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white"
            value={config.default_provider}
            onChange={(e) => setField('default_provider', e.target.value)}
          >
            <option value="" className="bg-slate-800">
              {enabledProviders.length === 1
                ? `${enabledProviders[0].name} (the only one)`
                : '— choose —'}
            </option>
            {enabledProviders.map((p) => (
              <option key={p.name} value={p.name} className="bg-slate-800">
                {p.name}
              </option>
            ))}
          </select>
          <Hint>
            System mail — magic-link sign-in above all — goes through this one, and so does any
            function that names no provider. With a single enabled provider it is implied.
          </Hint>
        </Field>
      </GlassCard>

      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold text-white">Providers</h2>
          <button
            onClick={addProvider}
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg bg-white/5 text-white/80 hover:text-white text-sm"
          >
            <Plus className="w-4 h-4" />
            Add provider
          </button>
        </div>

        {config.providers.length === 0 && (
          <GlassCard className="p-6 text-sm text-white/50">
            No providers yet. Add one — a Resend or Brevo API account, or any SMTP relay.
          </GlassCard>
        )}

        {config.providers.map((provider, index) => (
          <ProviderCard
            key={index}
            repo={repo ?? ''}
            branch={currentBranch}
            provider={provider}
            errors={errors.providers[index] ?? {}}
            isDefault={!!provider.name && provider.name === config.default_provider}
            secretPresent={secretsPresent[secretNameOf(provider.credential_ref) ?? '']}
            onChange={(patch) => patchProvider(index, patch)}
            onMakeDefault={() => setField('default_provider', provider.name)}
            onRemove={() => removeProvider(index)}
          />
        ))}
      </div>

      <GlassCard className="p-6 space-y-4">
        <div className="flex items-center gap-3">
          <button
            onClick={() => void save()}
            disabled={saving}
            className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-primary-500 text-white font-medium disabled:opacity-50"
          >
            <Send className="w-4 h-4" />
            {saving ? 'Saving…' : exists ? 'Save changes' : 'Create configuration'}
          </button>
          <Link
            to={`/${repo}/${currentBranch}/secrets`}
            className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-white/5 text-white/80 hover:text-white"
          >
            <KeyRound className="w-4 h-4" />
            Manage secrets
          </Link>
        </div>
      </GlassCard>

      <GlassCard className="p-6 space-y-4">
        <div>
          <h2 className="text-sm font-semibold text-white/80">Send a test message</h2>
          <p className="text-xs text-white/50">
            The only signal on this page that is not indirect. A saved config proves the fields
            are filled in and the secrets page proves a key EXISTS — nothing reads a key's value
            back, so the first proof that it is the RIGHT key is a real send. This goes through
            the same path a magic link does.
          </p>
        </div>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          <div className="md:col-span-2">
            <Input value={testTo} onChange={setTestTo} placeholder="you@example.com" />
          </div>
          <select
            className="bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white"
            value={testProvider}
            onChange={(e) => setTestProvider(e.target.value)}
          >
            <option value="" className="bg-slate-800">
              Default provider
            </option>
            {enabledProviders.map((p) => (
              <option key={p.name} value={p.name} className="bg-slate-800">
                {p.name}
              </option>
            ))}
          </select>
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={() => void sendTest()}
            disabled={testing || !testTo.trim() || !config.enabled || !testableHere}
            className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-white/10 text-white font-medium disabled:opacity-40"
          >
            <Send className="w-4 h-4" />
            {testing ? 'Sending…' : 'Send test'}
          </button>
          {!config.enabled && (
            <span className="text-xs text-amber-300">
              Enable and save the configuration first.
            </span>
          )}
          {config.enabled && !testableHere && (
            <span className="text-xs text-amber-300">
              Test sends run on <code>main</code>, so they would report on the wrong branch's
              configuration here.
            </span>
          )}
        </div>
        {testResult && (
          <p
            className={`text-xs ${testResult.ok ? 'text-emerald-300' : 'text-red-300'}`}
          >
            {testResult.ok ? '✓ ' : '✕ '}
            {testResult.detail}
          </p>
        )}
      </GlassCard>

      <GlassCard className="p-5 space-y-2">
        <h2 className="text-sm font-semibold text-white/80">Using it from a function</h2>
        <p className="text-xs text-white/50">
          One definition serves both runtimes, so QuickJS and Starlark see the same call. The
          sender identity and credential come from this page, never from the caller — a function
          chooses WHICH configured account to use, never who it appears to be.
        </p>
        <pre className="text-xs bg-black/30 rounded-lg p-3 text-white/70 overflow-x-auto">
{`// QuickJS — the tenant default (what a magic link uses)
await raisin.email.send({ to: ["a@example.com"], subject: "Hi", text: "..." })

// …or a named provider. An unknown name throws; it never falls back.
await raisin.email.send({ to: "a@example.com", subject: "Hi", text: "...",
                          provider: "marketing" })

// What is configured, without any credential
const { providers } = await raisin.email.providers()

# Starlark
raisin.email.send({"to": ["a@example.com"], "subject": "Hi", "text": "..."})`}
        </pre>
        <p className="text-xs text-white/50">
          Sending is denied by default per function: each one needs an{' '}
          <code className="text-white/70">email_policy</code> naming the recipients it may
          reach. Stored at <code className="text-white/70">{EMAIL_CONFIG_PATH}</code> in the{' '}
          <code className="text-white/70">raisin:system</code> workspace of this branch.
        </p>
      </GlassCard>

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}

function ProviderCard({
  repo,
  branch,
  provider,
  errors,
  isDefault,
  secretPresent,
  onChange,
  onMakeDefault,
  onRemove,
}: {
  repo: string
  branch: string
  provider: EmailProviderConfig
  errors: ProviderErrors
  isDefault: boolean
  secretPresent?: boolean
  onChange: (patch: Partial<EmailProviderConfig>) => void
  onMakeDefault: () => void
  onRemove: () => void
}) {
  const secretName = secretNameOf(provider.credential_ref)
  const smtp = provider.smtp ?? SMTP_DEFAULTS

  const changeKind = (kind: EmailProviderKind) => {
    // Switching TO smtp seeds the settings so the fields render filled in with
    // the submission defaults rather than blank-and-invalid. Switching away
    // keeps them: a mis-click must not delete a typed-in relay host.
    onChange({
      provider: kind,
      smtp: kind === 'smtp' ? (provider.smtp ?? { ...SMTP_DEFAULTS }) : provider.smtp,
    })
  }

  return (
    <GlassCard
      className={`p-6 space-y-5 ${isDefault ? 'border border-primary-400/40' : ''}`}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 grid grid-cols-1 md:grid-cols-2 gap-5">
          <Field label="Name" error={errors.name} required>
            <Input
              value={provider.name}
              onChange={(v) => onChange({ name: v })}
              placeholder="transactional"
            />
            <Hint>
              What a function passes as <code className="text-white/70">provider</code>. Not the
              API — two Resend accounts are two entries with two names.
            </Hint>
          </Field>
          <Field label="Provider API">
            <select
              className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white"
              value={provider.provider}
              onChange={(e) => changeKind(e.target.value as EmailProviderKind)}
            >
              {EMAIL_PROVIDERS.map((p) => (
                <option key={p} value={p} className="bg-slate-800">
                  {p}
                </option>
              ))}
            </select>
            <Hint>{PROVIDER_HELP[provider.provider]}</Hint>
          </Field>
        </div>
        <div className="flex flex-col items-end gap-2">
          <button
            onClick={onMakeDefault}
            disabled={isDefault || !provider.name.trim() || !provider.enabled}
            title={
              isDefault
                ? 'System mail goes through this provider'
                : 'Make this the provider system mail goes through'
            }
            className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs ${
              isDefault
                ? 'bg-primary-500/20 text-primary-200'
                : 'bg-white/5 text-white/60 hover:text-white disabled:opacity-40'
            }`}
          >
            <Star className={`w-3.5 h-3.5 ${isDefault ? 'fill-current' : ''}`} />
            {isDefault ? 'Default' : 'Make default'}
          </button>
          <button
            onClick={onRemove}
            title="Remove this provider"
            className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs bg-white/5 text-white/50 hover:text-red-300"
          >
            <Trash2 className="w-3.5 h-3.5" />
            Remove
          </button>
        </div>
      </div>

      <label className="flex items-center gap-2 cursor-pointer">
        <input
          type="checkbox"
          checked={provider.enabled}
          onChange={(e) => onChange({ enabled: e.target.checked })}
        />
        <span className="text-sm text-white/80">Enabled</span>
        <span className="text-xs text-white/40">
          A disabled provider cannot be selected — by name or as the default.
        </span>
      </label>

      <Field label="From address" error={errors.from_address} required>
        <Input
          value={provider.from_address}
          onChange={(v) => onChange({ from_address: v })}
          placeholder="no-reply@example.com"
        />
        <Hint>
          Must be on a domain verified with <em>this</em> account, or every send is rejected.
        </Hint>
      </Field>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-5">
        <Field label="From name" error={errors.from_name}>
          <Input
            value={provider.from_name ?? ''}
            onChange={(v) => onChange({ from_name: v })}
            placeholder="Example App"
          />
        </Field>
        <Field label="Reply-To" error={errors.reply_to}>
          <Input
            value={provider.reply_to ?? ''}
            onChange={(v) => onChange({ reply_to: v })}
            placeholder="support@example.com"
          />
        </Field>
      </div>

      {provider.provider === 'smtp' && (
        <div className="space-y-5 rounded-lg bg-black/20 p-4">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-5">
            <div className="md:col-span-2">
              <Field label="SMTP host" error={errors.smtp_host} required>
                <Input
                  value={smtp.host}
                  onChange={(v) => onChange({ smtp: { ...smtp, host: v } })}
                  placeholder="smtp-relay.brevo.com"
                />
              </Field>
            </div>
            <Field label="Port">
              <Input
                value={String(smtp.port)}
                onChange={(v) =>
                  onChange({ smtp: { ...smtp, port: Number(v.replace(/\D/g, '')) || 0 } })
                }
                placeholder="587"
              />
            </Field>
          </div>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-5">
            <Field label="Username">
              <Input
                value={smtp.username}
                onChange={(v) => onChange({ smtp: { ...smtp, username: v } })}
                placeholder="account@example.com"
              />
              <Hint>
                Leave empty only for a relay that authenticates by source address. The password
                is the secret referenced below.
              </Hint>
            </Field>
            <Field label="Security">
              <select
                className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white"
                value={smtp.security}
                onChange={(e) =>
                  onChange({ smtp: { ...smtp, security: e.target.value as SmtpSecurity } })
                }
              >
                {SMTP_SECURITY.map((s) => (
                  <option key={s} value={s} className="bg-slate-800">
                    {s}
                  </option>
                ))}
              </select>
              <Hint>{SECURITY_HELP[smtp.security]}</Hint>
            </Field>
          </div>
          <p className="text-xs text-white/40">
            The relay host must resolve to a public address. A loopback or private-network relay
            is refused unless the operator has enabled private egress server-wide — an
            operator-typed hostname is dialled by the server, which is the SSRF shape.
          </p>
        </div>
      )}

      <Field label="Credential reference" error={errors.credential_ref} required>
        <Input
          value={provider.credential_ref}
          onChange={(v) => onChange({ credential_ref: v })}
          placeholder="secret://email/api_key"
        />
        <Hint>
          A reference, never the key itself — a literal key here would be stored in a node
          property in the clear. Give each provider its own name so rotating one account cannot
          disturb another. Set the value under{' '}
          <Link className="underline" to={`/${repo}/${branch}/secrets`}>
            Secrets
          </Link>
          {secretName && (
            <>
              {' '}
              as <code className="text-white/70">{secretName}</code>
              {secretPresent === true && <span className="text-emerald-400"> — present</span>}
              {secretPresent === false && (
                <span className="text-red-400"> — missing, every send will fail</span>
              )}
            </>
          )}
          .
        </Hint>
      </Field>
    </GlassCard>
  )
}

function Field({
  label,
  error,
  required,
  children,
}: {
  label: string
  error?: string
  required?: boolean
  children: React.ReactNode
}) {
  return (
    <div className="space-y-1">
      <label className="text-sm text-white/80">
        {label}
        {required && <span className="text-amber-400"> *</span>}
      </label>
      {children}
      {error && <p className="text-xs text-red-300">{error}</p>}
    </div>
  )
}

function Input({
  value,
  onChange,
  placeholder,
}: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
}) {
  return (
    <input
      className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white placeholder-white/30"
      value={value}
      placeholder={placeholder}
      onChange={(e) => onChange(e.target.value)}
    />
  )
}

function Hint({ children }: { children: React.ReactNode }) {
  return <p className="text-xs text-white/50">{children}</p>
}
