// SPDX-License-Identifier: BSL-1.1

/**
 * Add a configured connector by instantiating a package's connector template.
 *
 * Why this exists at all: a package ships its connector as a TEMPLATE under
 * `/connectors`, and the operator's configured connector is a separate node
 * under `/integrations`. They used to be the same node, which meant the package
 * owned the operator's credentials — a re-install replaced the whole property
 * map and wiped the client id, the encrypted secret and every connected account.
 * Splitting them also makes two instances of one provider expressible (two
 * Microsoft tenants, each with its own client id and secret), which a single
 * shared node could not represent.
 *
 * This dialog only mints the instance. Credentials, the schema-driven connector
 * config and the connections list are all handled by `IntegrationEditor`, which
 * the caller opens on the new node — rather than rebuilding that machinery here.
 */

import { useMemo, useState } from 'react'
import { Plug, X } from 'lucide-react'
import { integrationsApi, type Integration } from '../../api/integrations'

interface Props {
  repo: string
  /** Templates shipped by installed adapter packages. */
  templates: Integration[]
  /** Existing instances, for the name-collision check. */
  existing: Integration[]
  onClose: () => void
  /** Called with the newly created instance so the caller can open the editor. */
  onCreated: (instance: Integration) => void
  onError: (title: string, message?: string) => void
}

const field =
  'w-full px-3 py-2 bg-white/5 border rounded-lg text-white placeholder-zinc-500 focus:outline-none'
const labelCls = 'block text-sm text-zinc-300 mb-1'

/** A node name must be a single safe path segment. */
function slugError(name: string, taken: Set<string>): string | null {
  if (!name.trim()) return 'A name is required.'
  if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) {
    return 'Use lowercase letters, numbers and hyphens, starting with a letter or number.'
  }
  if (taken.has(name)) return 'A connector with this name already exists.'
  return null
}

export default function AddConnectorDialog({
  repo,
  templates,
  existing,
  onClose,
  onCreated,
  onError,
}: Props) {
  const [picked, setPicked] = useState<Integration | null>(null)
  const [name, setName] = useState('')
  const [title, setTitle] = useState('')
  const [saving, setSaving] = useState(false)

  const taken = useMemo(() => new Set(existing.map((i) => i.name)), [existing])
  const nameError = slugError(name, taken)

  function pick(t: Integration) {
    setPicked(t)
    // Seed a sensible instance name, but never the template's own name — an
    // instance is a tenant/account, and the operator will usually run more than
    // one of them.
    setName(t.name)
    setTitle(t.title)
  }

  async function create() {
    if (!picked || nameError) return
    setSaving(true)
    try {
      const created = await integrationsApi.createIntegrationFromTemplate(repo, picked, {
        name: name.trim(),
        title: title.trim() || picked.title,
      })
      onCreated(created)
    } catch (e: any) {
      onError('Could not add connector', e?.message)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
      <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-2xl w-full max-h-[90vh] overflow-y-auto overscroll-contain">
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <h2 className="text-2xl font-bold text-white">Add connector</h2>
          <button onClick={onClose} className="p-2 hover:bg-white/10 rounded-lg transition-colors">
            <X className="w-5 h-5 text-white/60" />
          </button>
        </div>

        <div className="p-6 space-y-5">
          {templates.length === 0 ? (
            <p className="text-zinc-400 text-sm">
              No connector templates are available. Install an adapter package (category
              “integrations”) first — its template is what this dialog configures.
            </p>
          ) : (
            <>
              <div>
                <label className={labelCls}>Connector type *</label>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                  {templates.map((t) => {
                    const selected = picked?.path === t.path
                    return (
                      <button
                        type="button"
                        key={t.path || t.name}
                        onClick={() => pick(t)}
                        className={`flex items-center gap-3 px-4 py-3 rounded-lg border text-left transition-colors ${
                          selected
                            ? 'bg-primary-500/10 border-primary-500/50'
                            : 'bg-white/5 border-white/10 hover:bg-white/10'
                        }`}
                      >
                        <Plug className="w-5 h-5 text-sky-400 flex-shrink-0" />
                        <div className="min-w-0">
                          <div className="text-white text-sm truncate">{t.title || t.name}</div>
                          <div className="text-zinc-500 text-xs truncate">{t.provider_type}</div>
                        </div>
                      </button>
                    )
                  })}
                </div>
              </div>

              {picked && (
                <>
                  <div className="grid grid-cols-2 gap-4">
                    <div>
                      <label className={labelCls}>Name *</label>
                      <input
                        className={`${field} ${
                          nameError
                            ? 'border-red-500/50 focus:border-red-500'
                            : 'border-white/10 focus:border-primary-500'
                        }`}
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="contoso-tenant"
                      />
                      {nameError ? (
                        <p className="mt-1 text-xs text-red-400">{nameError}</p>
                      ) : (
                        <p className="mt-1 text-xs text-zinc-500">
                          Give each account or tenant its own name — you can add this connector
                          more than once.
                        </p>
                      )}
                    </div>
                    <div>
                      <label className={labelCls}>Title *</label>
                      <input
                        className={`${field} border-white/10 focus:border-primary-500`}
                        value={title}
                        onChange={(e) => setTitle(e.target.value)}
                        placeholder="Contoso (Microsoft 365)"
                      />
                    </div>
                  </div>

                  <p className="text-xs text-zinc-500">
                    The provider settings, config schema and setup instructions come from the
                    template. You will add the client id, client secret and connections next.
                  </p>
                </>
              )}
            </>
          )}
        </div>

        <div className="flex justify-end gap-2 p-6 border-t border-white/10">
          <button
            onClick={onClose}
            className="px-4 py-2 text-zinc-300 hover:bg-white/10 rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={create}
            disabled={!picked || !!nameError || saving}
            className="px-4 py-2 bg-primary-500 hover:bg-primary-600 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
          >
            {saving ? 'Adding…' : 'Add connector'}
          </button>
        </div>
      </div>
    </div>
  )
}
