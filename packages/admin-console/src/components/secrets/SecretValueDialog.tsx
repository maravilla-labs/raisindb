// SPDX-License-Identifier: BSL-1.1

/**
 * The one place a secret value is typed. Create and rotate share it.
 *
 * The input is WRITE-ONLY in the strict sense: it is never seeded from the
 * server (no route returns a value), and the draft is dropped the moment the
 * submit resolves or the dialog closes, so a plaintext never outlives the
 * request that carried it. There is no reveal and no copy affordance, because
 * there is nothing to reveal — the field is empty until the operator types.
 */

import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { KeyRound, X } from 'lucide-react'

interface SecretValueDialogProps {
  open: boolean
  mode: 'create' | 'rotate'
  /** Fixed on rotate; editable on create. */
  name?: string
  busy?: boolean
  onSubmit: (name: string, value: string) => void
  onCancel: () => void
}

export default function SecretValueDialog({
  open,
  mode,
  name,
  busy = false,
  onSubmit,
  onCancel,
}: SecretValueDialogProps) {
  const [draftName, setDraftName] = useState('')
  const [value, setValue] = useState('')

  // Clearing on every open/close transition is what keeps a typed plaintext
  // from surviving a cancelled dialog and reappearing in the next one.
  useEffect(() => {
    setDraftName(mode === 'rotate' ? name || '' : '')
    setValue('')
  }, [open, mode, name])

  if (!open) return null

  const effectiveName = mode === 'rotate' ? name || '' : draftName.trim()
  const canSubmit = !busy && effectiveName.length > 0 && value.length > 0

  const submit = () => {
    if (!canSubmit) return
    onSubmit(effectiveName, value)
    // Drop the plaintext immediately; the request already holds it.
    setValue('')
  }

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div className="glass w-full max-w-lg rounded-xl p-6 space-y-4">
        <div className="flex items-start justify-between">
          <h2 className="text-lg font-semibold text-white flex items-center gap-2">
            <KeyRound className="w-5 h-5 text-primary-400" />
            {mode === 'create' ? 'New secret' : `Rotate ${name}`}
          </h2>
          <button
            type="button"
            onClick={onCancel}
            className="text-zinc-400 hover:text-white"
            aria-label="Close"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {mode === 'create' ? (
          <div className="space-y-1">
            <label className="text-xs text-zinc-400">Name</label>
            <input
              value={draftName}
              onChange={(e) => setDraftName(e.target.value)}
              placeholder="stripe_api_key"
              autoComplete="off"
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm font-mono focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
            />
            <p className="text-xs text-zinc-500">
              Referenced from a property as <code className="font-mono">secret://{effectiveName || 'name'}</code>.
              Names beginning <code className="font-mono">node/</code> belong to the auto-vault and
              should not be created by hand.
            </p>
          </div>
        ) : (
          <p className="text-xs text-zinc-500">
            A rotation APPENDS a version. Anything holding a pinned{' '}
            <code className="font-mono">secret://{name}@N</code> — an older node revision, a running
            flow — keeps resolving the old value.
          </p>
        )}

        <div className="space-y-1">
          <label className="text-xs text-zinc-400">Value</label>
          <textarea
            value={value}
            onChange={(e) => setValue(e.target.value)}
            rows={3}
            autoComplete="off"
            spellCheck={false}
            placeholder="Paste the value — it is sealed on write and never shown again"
            className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm font-mono focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
          />
          <p className="text-xs text-amber-200/70">
            Write-only. The server has no route that returns a value, so this cannot be read back
            here or anywhere else — only used server-side.
          </p>
        </div>

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="px-3 py-1.5 rounded-md border border-white/10 text-sm text-zinc-300"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={!canSubmit}
            className="px-3 py-1.5 rounded-md bg-primary-500/20 border border-primary-400/40 text-white text-sm disabled:opacity-40"
          >
            {mode === 'create' ? 'Create' : 'Rotate'}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  )
}
