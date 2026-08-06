// SPDX-License-Identifier: BSL-1.1

import { Info } from 'lucide-react'
import type { Capabilities, MountState, WriteConfig } from '../../api/integrations'

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500'
const labelCls = 'block text-white text-sm font-medium mb-1.5'

interface Props {
  value: WriteConfig
  /** Patch, never replace — unknown `write_config` keys must survive an edit. */
  onChange: (patch: Partial<WriteConfig>) => void
  caps?: Capabilities
  capsUnknown: boolean
  state?: MountState
}

/**
 * The mount's write configuration, extracted from `MountEditor` (already far
 * over the 300-line convention) following the `MountPushPanel` precedent.
 *
 * Only the two modes the engine implements are offered. `mirror` and `submit`
 * parse but are REFUSED by the engine, so offering them would produce a mount
 * that silently does nothing — the exact failure this fieldset exists to end.
 * `writeback` (the legacy mode-less mirror switch) is deliberately not editable
 * here for the same reason; it round-trips untouched.
 */
export default function MountWriteFieldset({ value, onChange, caps, capsUnknown, state }: Props) {
  const mode = value.mode === 'state_only' ? 'state_only' : 'off'
  const selected = value.mutable_fields || []
  // What the provider says it accepts. Empty is a normal state — an adapter may
  // implement `update` without enumerating fields — so we fall back to free text
  // rather than locking the operator out.
  const offered = caps?.mutable_fields || []
  // Exactly what the engine's `missing_state_only_ops` requires — `can_write`
  // (the umbrella flag) plus `can_update`. Naming the missing ops here is what
  // stops "this connector is read-only" being the only explanation an operator
  // ever gets; the engine's own refusal names the same two.
  const missingOps = [
    ...(caps?.can_write === true ? [] : ['can_write']),
    ...(caps?.can_update === true ? [] : ['can_update']),
  ]
  // Conservative-unknown, the same invariant the rest of the editor holds: an
  // unprobed connector is treated as read-only, never as permissive.
  const disabled = capsUnknown || missingOps.length > 0

  function toggleField(name: string, on: boolean) {
    const next = on ? [...new Set([...selected, name])] : selected.filter((f) => f !== name)
    onChange({ mutable_fields: next })
  }

  return (
    <fieldset className="border border-white/10 rounded-lg p-4 space-y-3">
      <legend className="px-2 text-sm font-semibold text-zinc-300">Writeback</legend>

      <div>
        <label className={labelCls}>Mode</label>
        <select
          className={field}
          disabled={disabled}
          value={mode}
          onChange={(e) => onChange({ mode: e.target.value as WriteConfig['mode'] })}
        >
          <option value="off">off (read-only)</option>
          <option value="state_only">state_only (push listed properties)</option>
        </select>
        {disabled && (
          <p className="flex items-start gap-1.5 text-xs text-zinc-500 mt-1.5">
            <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
            {capsUnknown
              ? 'Capabilities unknown — run Test connection. This mount stays read-only until then.'
              : `This connector cannot push local edits — it does not declare ${missingOps.join(' or ')}.`}
          </p>
        )}
      </div>

      {mode === 'state_only' && (
        <div>
          <label className={labelCls}>Mutable fields</label>
          {offered.length > 0 ? (
            <div className="flex flex-wrap gap-x-4 gap-y-1.5">
              {offered.map((f) => (
                <label key={f} className="flex items-center gap-2 text-white text-sm">
                  <input
                    type="checkbox"
                    className="w-4 h-4 rounded"
                    checked={selected.includes(f)}
                    onChange={(e) => toggleField(f, e.target.checked)}
                  />
                  <span className="font-mono text-xs">{f}</span>
                </label>
              ))}
            </div>
          ) : (
            <textarea
              className={field}
              rows={3}
              placeholder={'unread\nflagged'}
              value={selected.join('\n')}
              onChange={(e) =>
                onChange({
                  mutable_fields: e.target.value
                    .split(/[\n,]/)
                    .map((s) => s.trim())
                    .filter(Boolean),
                })
              }
            />
          )}
          <p className="flex items-start gap-1.5 text-xs text-zinc-500 mt-1.5">
            <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
            {offered.length > 0
              ? 'Only the properties this provider declared it accepts. Everything else stays read-only.'
              : 'This connector did not enumerate its writable properties — one property name per line.'}
          </p>
          {selected.length === 0 && (
            <p className="text-xs text-amber-400 mt-1">
              No fields selected — the engine refuses writeback with “mount declares no
              write_config.mutable_fields”.
            </p>
          )}
        </div>
      )}

      {mode !== 'off' && (
        <div>
          <label className={labelCls}>Conflict</label>
          <select
            className={field}
            value={value.conflict || 'remote_wins'}
            onChange={(e) => onChange({ conflict: e.target.value as WriteConfig['conflict'] })}
          >
            <option value="remote_wins">remote_wins</option>
            <option value="error">error</option>
          </select>
        </div>
      )}

      {/*
        The engine's own verdict, read-only. It is recomputed on every sync run
        from the adapter and the mapper, so a mode just saved here shows its
        verdict only after the next run — say so, or a stale "not supported"
        reads as a rejection of the change that was just made.
      */}
      <dl className="space-y-1.5 pt-1 border-t border-white/5">
        <Row label="Engine">
          {state?.writeback_supported === true ? (
            <span className="text-green-400 text-xs">supported</span>
          ) : state?.writeback_supported === false ? (
            <span className="text-red-400 text-xs">not supported</span>
          ) : (
            <span className="text-zinc-400 text-xs">unknown — not yet evaluated</span>
          )}
        </Row>
        {state?.writeback_last_error && (
          <Row label="Reason">
            <span className="text-red-400 text-xs break-words">{state.writeback_last_error}</span>
          </Row>
        )}
      </dl>
      <p className="text-xs text-zinc-500">
        The engine re-evaluates this on the next sync run, not on save.
      </p>
    </fieldset>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-3">
      <dt className="text-zinc-500 text-xs w-24 shrink-0">{label}</dt>
      <dd className="min-w-0 flex-1">{children}</dd>
    </div>
  )
}
