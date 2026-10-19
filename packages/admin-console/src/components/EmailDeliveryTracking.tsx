// SPDX-License-Identifier: BSL-1.1

/**
 * Self-service setup of a sender's delivery-feedback webhook.
 *
 * Sending proves only that Brevo or Resend ACCEPTED a message. This panel wires
 * up the channel they use to say what happened next (delivered, bounced,
 * blocked, complained): one click mints the webhook URL and registers it at the
 * provider with the sender's own API key. When registration is not possible —
 * a restricted key, a server the provider cannot reach — it falls back to the
 * URL to paste by hand.
 */

import { useCallback, useEffect, useState } from 'react'
import { Check, Copy, Radio } from 'lucide-react'
import {
  emailWebhookApi,
  type EmailProviderKind,
  type WebhookSetupResult,
  type WebhookStatus,
} from '../api/email-config'

const MANUAL_STEPS: Partial<Record<EmailProviderKind, string>> = {
  brevo:
    'In Brevo: Transactional → Settings → Webhook → Add a new webhook. Paste the URL and tick ' +
    'delivered, deferred, soft/hard bounce, blocked, spam, error and unsubscribed (opened/clicked optional).',
  resend:
    'In Resend: Webhooks → Add Endpoint. Paste the URL, select the email.* events, then copy the ' +
    'endpoint\'s signing secret (whsec_…) into the field below.',
}

export default function EmailDeliveryTracking({
  repo,
  name,
  kind,
}: {
  repo: string
  name: string
  kind: EmailProviderKind
}) {
  const [status, setStatus] = useState<WebhookStatus | null>(null)
  const [result, setResult] = useState<WebhookSetupResult | null>(null)
  const [notSaved, setNotSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [signing, setSigning] = useState('')
  const [copied, setCopied] = useState(false)

  const refresh = useCallback(async () => {
    if (!name) return
    try {
      setStatus(await emailWebhookApi.status(repo, name))
      setNotSaved(false)
    } catch (e) {
      // The sender is not in the SAVED config yet: nothing to attach a webhook to.
      if ((e as { status?: number })?.status === 404) setNotSaved(true)
      else setError((e as Error).message)
    }
  }, [repo, name])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const run = async (opts: Parameters<typeof emailWebhookApi.setup>[2]) => {
    setBusy(true)
    setError(null)
    try {
      setResult(await emailWebhookApi.setup(repo, name, opts))
      setSigning('')
      await refresh()
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  const url = result?.url ?? status?.url ?? null
  const live = !!status?.configured
  const needsSecret = (status?.needs_signing_secret ?? kind === 'resend') && !status?.signing_secret_set
  const warning = result?.warning ?? status?.warning

  return (
    <div className="border-t border-white/10 pt-4 space-y-3">
      <div className="flex items-center gap-2 text-sm font-medium text-white">
        <Radio className="w-4 h-4" />
        Delivery tracking
        {live && !needsSecret && <span className="text-emerald-400 text-xs">— active</span>}
      </div>
      <p className="text-xs text-white/50">
        Without this, “sent” only means the provider accepted the message. With it, bounces,
        blocks and complaints are recorded per message as they happen.
      </p>

      {notSaved ? (
        <p className="text-xs text-amber-300">Save the configuration first, then set this up.</p>
      ) : (
        <>
          {!live && (
            <button
              disabled={busy || !name}
              onClick={() => void run({ register: true })}
              className="px-3 py-1.5 rounded-lg bg-primary-500 text-white text-sm disabled:opacity-50"
            >
              {busy ? 'Setting up…' : 'Enable delivery tracking'}
            </button>
          )}

          {result?.registration_error && (
            <p className="text-xs text-amber-300">
              Automatic registration failed: {result.registration_error}. Use the URL below to
              add it manually.
            </p>
          )}
          {result?.registered && !result.registration_error && (
            <p className="text-xs text-emerald-400">Webhook created at the provider.</p>
          )}
          {warning && <p className="text-xs text-amber-300">{warning}</p>}
          {error && <p className="text-xs text-red-400">{error}</p>}

          {live && url && (
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <code className="flex-1 truncate text-xs bg-white/5 rounded px-2 py-1.5 text-white/70">
                  {url}
                </code>
                <button
                  onClick={() => {
                    void navigator.clipboard.writeText(url)
                    setCopied(true)
                    setTimeout(() => setCopied(false), 1500)
                  }}
                  className="p-1.5 rounded bg-white/5 text-white/70 hover:text-white"
                  title="Copy URL"
                >
                  {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
                </button>
              </div>
              {!status?.registered && (
                <p className="text-xs text-white/50">{MANUAL_STEPS[kind]}</p>
              )}
            </div>
          )}

          {live && needsSecret && (
            <div className="space-y-1">
              <p className="text-xs text-amber-300">
                Resend events are rejected until the signing secret is stored.
              </p>
              <div className="flex gap-2">
                <input
                  type="password"
                  value={signing}
                  onChange={(e) => setSigning(e.target.value)}
                  placeholder="whsec_…"
                  className="flex-1 bg-white/5 border border-white/10 rounded-lg px-3 py-1.5 text-sm text-white"
                />
                <button
                  disabled={busy || !signing.trim()}
                  onClick={() => void run({ signing_secret: signing })}
                  className="px-3 py-1.5 rounded-lg bg-white/10 text-white text-sm disabled:opacity-50"
                >
                  Save secret
                </button>
              </div>
            </div>
          )}

          {live && (
            <button
              disabled={busy}
              onClick={() => void run({ rotate: true, register: status?.registered })}
              className="text-xs text-white/40 hover:text-white/70 underline"
            >
              Rotate URL (the old one stops working)
            </button>
          )}
        </>
      )}
    </div>
  )
}
