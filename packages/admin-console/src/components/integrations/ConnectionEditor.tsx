// SPDX-License-Identifier: BSL-1.1

import { useEffect, useMemo, useState } from 'react'
import { X, Loader2, Info } from 'lucide-react'
import SchemaForm from '../PropertyFields/SchemaForm'
import { nodeTypesApi, type ResolvedNodeType } from '../../api/nodetypes'
import {
  integrationsApi,
  SYSTEM_WORKSPACE,
  CONFIG_BRANCH,
  type Connection,
  type Integration,
} from '../../api/integrations'
import { getDefaultValueForSchema, isSecretField } from '../../utils/propertySchema'

interface ConnectionEditorProps {
  repo: string
  integration: Integration
  /** Editing an existing connection; omit to create one. */
  connection?: Connection
  onClose: () => void
  onSaved: () => void
  onError: (title: string, message?: string) => void
  onSuccess: (title: string, message?: string) => void
}

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500'
const labelCls = 'block text-xs font-medium text-zinc-400 mb-1'

/**
 * Add or edit one connection on a connector.
 *
 * The form is generated from the connector's `connection_config_type`, so a new
 * connector ships its own fields without any console change. This is the
 * non-OAuth path to a connected account — the one that makes IMAP usable — and
 * it is also what lets a single connector hold several connections.
 */
export default function ConnectionEditor({
  repo,
  integration,
  connection,
  onClose,
  onSaved,
  onError,
  onSuccess,
}: ConnectionEditorProps) {
  const isEdit = !!connection
  const [resolved, setResolved] = useState<ResolvedNodeType | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [label, setLabel] = useState(connection?.label || '')
  const [values, setValues] = useState<Record<string, any>>(
    (connection?.config as Record<string, any>) || {}
  )
  const [secretDrafts, setSecretDrafts] = useState<Record<string, string | null>>({})
  const [saving, setSaving] = useState(false)

  const typeName = integration.connection_config_type

  useEffect(() => {
    if (!typeName) {
      setLoading(false)
      return
    }
    let cancelled = false
    nodeTypesApi
      .getResolved(repo, CONFIG_BRANCH, typeName, SYSTEM_WORKSPACE)
      .then((r) => {
        if (cancelled) return
        setResolved(r)
        // Seed defaults for a NEW connection only; editing must not silently
        // rewrite stored values the operator did not touch.
        if (!isEdit) {
          const seeded: Record<string, any> = {}
          for (const p of r.resolved_properties || []) {
            const name = (p as any).name
            if (!name || isSecretField(p)) continue
            const d = getDefaultValueForSchema(p)
            if (d !== undefined) seeded[name] = d
          }
          setValues(seeded)
        }
      })
      .catch((e: any) => {
        if (!cancelled) setLoadError(e?.message || 'Could not load the connection schema')
      })
      .finally(() => !cancelled && setLoading(false))
    return () => {
      cancelled = true
    }
  }, [repo, typeName, isEdit])

  const properties = useMemo(
    () => (resolved?.resolved_properties as any[]) || [],
    [resolved]
  )

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!integration.path) return
    setSaving(true)
    try {
      // Only send secrets the operator actually touched — an untouched field
      // must keep its stored value, and we never had the value to resend.
      const secrets = Object.fromEntries(
        Object.entries(secretDrafts).filter(([, v]) => v === null || (v && v.length > 0))
      )
      const body = {
        label: label.trim() || undefined,
        config: values,
        ...(Object.keys(secrets).length > 0 ? { secrets } : {}),
      }
      if (isEdit) {
        await integrationsApi.updateConnection(repo, integration.path, connection!.id, body)
      } else {
        await integrationsApi.createConnection(repo, integration.path, body)
      }
      onSuccess(isEdit ? 'Connection updated' : 'Connection added', label.trim() || undefined)
      onSaved()
      onClose()
    } catch (e: any) {
      onError('Save failed', e?.message)
    } finally {
      setSaving(false)
    }
  }

  const storedSecrets = connection?.secret_fields || []

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-zinc-900 border border-white/10 rounded-xl w-full max-w-2xl max-h-[90vh] overflow-y-auto">
        <div className="flex items-center justify-between px-6 py-4 border-b border-white/10">
          <h2 className="text-lg font-semibold text-white">
            {isEdit ? 'Edit connection' : 'Add connection'}
          </h2>
          <button type="button" onClick={onClose} className="text-zinc-400 hover:text-white">
            <X className="w-5 h-5" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="p-6 space-y-5">
          <div>
            <label className={labelCls}>Connection name</label>
            <input
              className={field}
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="Support mailbox"
            />
          </div>

          {loading && (
            <div className="flex items-center gap-2 text-sm text-zinc-400">
              <Loader2 className="w-4 h-4 animate-spin" /> Loading connection fields…
            </div>
          )}

          {!loading && !typeName && (
            <div className="flex gap-2 rounded-lg border border-white/10 bg-white/5 p-3 text-xs text-zinc-400">
              <Info className="w-4 h-4 flex-shrink-0 text-zinc-500" />
              <span>
                This connector declares no <code>connection_config_type</code>, so it has no
                per-connection fields. Connect an account through OAuth instead, or upgrade the
                adapter package.
              </span>
            </div>
          )}

          {!loading && loadError && (
            <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-xs text-amber-300">
              Could not load <code>{typeName}</code>: {loadError}. The adapter package that
              declares it may not be installed in this repository.
            </div>
          )}

          {!loading && properties.length > 0 && (
            <SchemaForm
              properties={properties}
              values={values}
              secretsSet={storedSecrets}
              secretDrafts={secretDrafts}
              onChange={(name, value) => setValues((prev) => ({ ...prev, [name]: value }))}
              onSecretChange={(name, value) =>
                setSecretDrafts((prev) => ({ ...prev, [name]: value }))
              }
              disabled={saving}
            />
          )}

          <div className="flex flex-col-reverse md:flex-row gap-3 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="flex-1 px-6 py-3 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving || loading}
              className="flex-1 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {saving ? 'Saving…' : isEdit ? 'Save connection' : 'Add connection'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
