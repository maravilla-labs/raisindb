// SPDX-License-Identifier: BSL-1.1

/**
 * Outbound (transactional) email configuration, per repository branch.
 *
 * The tenant brings their own Resend or Brevo account and verifies their own
 * sending domain; mail goes out as them. There is deliberately no Maravilla
 * fallback sender, so a tenant who has configured nothing cannot send at all —
 * rather than sending as somebody else.
 *
 * Two states here look fine and are not, so both are called out in the UI:
 *
 *  - **Disabled.** The server treats an absent or non-`true` `enabled` as off.
 *    That is the correct default (a verified sending domain comes first), but a
 *    saved, complete-looking config that still sends nothing is confusing
 *    without being told why.
 *  - **No API key.** The config names a `secret://` reference; the key itself
 *    lives in the secret store and nothing on this page can see it. A config
 *    can therefore be valid, enabled, and still fail on the first send.
 */

import { useCallback, useEffect, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { AlertTriangle, KeyRound, Mail, Send } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { useToast, ToastContainer } from '../components/Toast'
import {
  EMAIL_CONFIG_DEFAULTS,
  EMAIL_CONFIG_PATH,
  EMAIL_PROVIDERS,
  emailConfigApi,
  secretNameOf,
  validate,
  type EmailConfig,
  type EmailProvider,
} from '../api/email-config'
import { secretsApi } from '../api/secrets'

const PROVIDER_HELP: Record<EmailProvider, string> = {
  resend: 'API key from resend.com — the sending domain must be verified there.',
  brevo: 'API key (v3) from brevo.com — the sender must be verified there.',
}

export default function EmailSettings() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const currentBranch = branch || 'main'

  const [config, setConfig] = useState<EmailConfig>(EMAIL_CONFIG_DEFAULTS)
  const [exists, setExists] = useState(false)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [errors, setErrors] = useState<Partial<Record<keyof EmailConfig, string>>>({})
  // Whether the referenced secret exists. `null` = not yet determined, so the
  // warning is not flashed before we know.
  const [secretPresent, setSecretPresent] = useState<boolean | null>(null)
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

  // Existence only — `list` is metadata, and there is no route that returns a
  // secret's value.
  useEffect(() => {
    if (!repo) return
    const name = secretNameOf(config.credential_ref)
    if (!name) {
      setSecretPresent(null)
      return
    }
    let cancelled = false
    secretsApi
      .list(repo, currentBranch)
      .then((all) => {
        if (cancelled) return
        setSecretPresent(all.some((s) => s.name === name && !s.deleted))
      })
      .catch(() => {
        // A failed probe must not render as "missing" — that would send an
        // operator hunting for a secret that is present.
        if (!cancelled) setSecretPresent(null)
      })
    return () => {
      cancelled = true
    }
  }, [repo, currentBranch, config.credential_ref])

  const set = <K extends keyof EmailConfig>(key: K, value: EmailConfig[K]) => {
    setConfig((c) => ({ ...c, [key]: value }))
    setErrors((e) => ({ ...e, [key]: undefined }))
  }

  const save = async () => {
    if (!repo) return
    const found = validate(config)
    setErrors(found)
    if (Object.keys(found).length > 0) {
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

  const secretName = secretNameOf(config.credential_ref)

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
            provider account, from your own verified domain.
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
              configuration error. Enable it once your sending domain is verified with the
              provider.
            </p>
          </div>
        </GlassCard>
      )}

      {config.enabled && secretPresent === false && secretName && (
        <GlassCard className="p-4 border border-red-400/40">
          <div className="flex gap-3">
            <AlertTriangle className="w-5 h-5 text-red-400 flex-shrink-0" />
            <p className="text-sm text-red-200">
              Email is enabled but the secret <code className="text-red-100">{secretName}</code>{' '}
              does not exist on this branch. Every send will fail. Create it under{' '}
              <Link className="underline" to={`/${repo}/${currentBranch}/secrets`}>
                Secrets
              </Link>
              .
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
            onChange={(e) => set('enabled', e.target.checked)}
          />
          <span>
            <span className="text-white font-medium">Enable outbound email</span>
            <span className="block text-xs text-white/50">
              Leave off until the sending domain is verified with your provider.
            </span>
          </span>
        </label>

        <Field label="Provider" error={errors.provider}>
          <select
            className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-white"
            value={config.provider}
            onChange={(e) => set('provider', e.target.value as EmailProvider)}
          >
            {EMAIL_PROVIDERS.map((p) => (
              <option key={p} value={p} className="bg-slate-800">
                {p}
              </option>
            ))}
          </select>
          <Hint>{PROVIDER_HELP[config.provider]}</Hint>
        </Field>

        <Field label="From address" error={errors.from_address} required={config.enabled}>
          <Input
            value={config.from_address}
            onChange={(v) => set('from_address', v)}
            placeholder="no-reply@example.com"
          />
          <Hint>Must be on a domain verified with the provider, or every send is rejected.</Hint>
        </Field>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-5">
          <Field label="From name" error={errors.from_name}>
            <Input
              value={config.from_name ?? ''}
              onChange={(v) => set('from_name', v)}
              placeholder="Example App"
            />
          </Field>
          <Field label="Reply-To" error={errors.reply_to}>
            <Input
              value={config.reply_to ?? ''}
              onChange={(v) => set('reply_to', v)}
              placeholder="support@example.com"
            />
          </Field>
        </div>

        <Field label="Front-end base URL" error={errors.base_url} required={config.enabled}>
          <Input
            value={config.base_url}
            onChange={(v) => set('base_url', v)}
            placeholder="https://app.example.com"
          />
          <Hint>
            Magic links are built from this. A wrong value produces links that work but point
            at the wrong host.
          </Hint>
        </Field>

        <Field label="Credential reference" error={errors.credential_ref} required>
          <Input
            value={config.credential_ref}
            onChange={(v) => set('credential_ref', v)}
            placeholder="secret://email/api_key"
          />
          <Hint>
            A reference, never the key itself — a literal key here would be stored in a node
            property in the clear. Set the value under{' '}
            <Link className="underline" to={`/${repo}/${currentBranch}/secrets`}>
              Secrets
            </Link>
            {secretName && (
              <>
                {' '}
                as <code className="text-white/70">{secretName}</code>
                {secretPresent === true && <span className="text-emerald-400"> — present</span>}
              </>
            )}
            .
          </Hint>
        </Field>

        <div className="flex items-center gap-3 pt-2">
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

      <GlassCard className="p-5 space-y-2">
        <h2 className="text-sm font-semibold text-white/80">Using it from a function</h2>
        <p className="text-xs text-white/50">
          One definition serves both runtimes, so QuickJS and Starlark see the same call. The
          sender identity and credential come from this page, never from the caller.
        </p>
        <pre className="text-xs bg-black/30 rounded-lg p-3 text-white/70 overflow-x-auto">
{`// QuickJS
raisin.email.send({ to: ["a@example.com"], subject: "Hi", text: "..." })

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
