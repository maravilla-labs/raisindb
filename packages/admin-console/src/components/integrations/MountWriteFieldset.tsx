// SPDX-License-Identifier: BSL-1.1

import { Info } from 'lucide-react'
import type { Capabilities, MountState, WriteConfig } from '../../api/integrations'
import MountWriteRails from './MountWriteRails'

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
  /**
   * The mount's `resolver_function` path. A MOUNT property, not a
   * `write_config` key, so it is threaded separately rather than folded into
   * `value` — the round-trip that preserves unknown `write_config` keys would
   * otherwise write it into the wrong object and the engine would never see it.
   */
  resolverFunction: string
  onResolverChange: (path: string) => void
}

/** Which capabilities the engine's own resolver demands, per mode. */
const MODES = [
  {
    value: 'off',
    label: 'off (read-only)',
    /** Extra ops beyond `can_write`, matching `write::plan::resolve`. */
    ops: [] as (keyof Capabilities)[],
  },
  {
    value: 'state_only',
    label: 'state_only — push listed properties',
    ops: ['can_update'] as (keyof Capabilities)[],
  },
  {
    value: 'mirror',
    label: 'mirror — the node IS the remote object',
    // `can_create` is NOT demanded here: the engine asks for it only from a
    // mount that opted node types into local creation, so requiring it would
    // hide `mirror` from every adapter that updates and deletes perfectly well.
    // `MountWriteRails` raises it at the point the operator opts in.
    ops: ['can_update', 'can_delete'] as (keyof Capabilities)[],
  },
  {
    value: 'submit',
    label: 'submit — the node is a command',
    ops: ['can_submit'] as (keyof Capabilities)[],
  },
] as const

/**
 * The mount's write configuration, extracted from `MountEditor` (already far
 * over the 300-line convention) following the `MountPushPanel` precedent.
 *
 * Every mode the engine implements is offered, and each is gated on exactly the
 * capabilities the engine's own resolver demands for it — a mode the connector
 * cannot serve is disabled WITH ITS REASON rather than hidden, because "this
 * connector is read-only" is not an answer an operator can act on. The gate must
 * keep matching `write::plan::resolve`: a mode offered here that the engine
 * refuses produces a mount that silently does nothing, which is the exact
 * failure this fieldset exists to end.
 *
 * `writeback` (the legacy mode-less spelling of `mirror`) stays uneditable and
 * round-trips untouched — two controls for one setting is how they drift.
 */
export default function MountWriteFieldset({
  value,
  onChange,
  caps,
  capsUnknown,
  state,
  resolverFunction,
  onResolverChange,
}: Props) {
  const mode = MODES.some((m) => m.value === value.mode)
    ? (value.mode as (typeof MODES)[number]['value'])
    : 'off'
  const selected = value.mutable_fields || []
  // What the provider says it accepts. Empty is a normal state — an adapter may
  // implement `update` without enumerating fields — so we fall back to free text
  // rather than locking the operator out.
  const offered = caps?.mutable_fields || []
  // Which ops a given mode needs and this connector has not declared. Naming
  // them is what stops "this connector is read-only" being the only explanation
  // an operator ever gets; the engine's own refusal names the same ones.
  function missingFor(m: (typeof MODES)[number]): string[] {
    if (m.value === 'off') return []
    return [
      ...(caps?.can_write === true ? [] : ['can_write']),
      ...m.ops.filter((op) => caps?.[op] !== true).map(String),
    ]
  }
  const missingOps = missingFor(MODES.find((m) => m.value === mode) ?? MODES[0])
  // Conservative-unknown, the same invariant the rest of the editor holds: an
  // unprobed connector is treated as read-only, never as permissive.
  const disabled = capsUnknown || (mode !== 'off' && missingOps.length > 0)
  // `mutable_fields` is read by state_only and mirror alike; submit pushes no
  // field list at all, because a command is not a patch of anything.
  const usesFields = mode === 'state_only' || mode === 'mirror'

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
          disabled={capsUnknown}
          value={mode}
          onChange={(e) => onChange({ mode: e.target.value as WriteConfig['mode'] })}
        >
          {MODES.map((m) => {
            const missing = missingFor(m)
            return (
              <option key={m.value} value={m.value} disabled={missing.length > 0}>
                {m.label}
                {missing.length > 0 ? ` — needs ${missing.join(' + ')}` : ''}
              </option>
            )
          })}
        </select>
        {disabled && (
          <p className="flex items-start gap-1.5 text-xs text-zinc-500 mt-1.5">
            <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
            {capsUnknown
              ? 'Capabilities unknown — run Test connection. This mount stays read-only until then.'
              : `This connector cannot serve “${mode}” — it does not declare ${missingOps.join(' or ')}.`}
          </p>
        )}
      </div>

      {usesFields && (
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
              {mode === 'mirror'
                ? 'No fields selected — local edits will not push. A mirror whose only outbound effect is create/delete is a valid configuration, so the engine allows it.'
                : 'No fields selected — the engine refuses writeback with “mount declares no write_config.mutable_fields”.'}
            </p>
          )}
        </div>
      )}

      {mode === 'mirror' && <MountWriteRails value={value} onChange={onChange} caps={caps} />}

      {mode === 'submit' && (
        <div>
          <label className={labelCls}>Command node types</label>
          <textarea
            className={field}
            rows={2}
            placeholder={'raisin:OutboundMail'}
            value={(value.command_node_types || []).join('\n')}
            onChange={(e) =>
              onChange({
                command_node_types: e.target.value
                  .split(/[\n,]/)
                  .map((s) => s.trim())
                  .filter(Boolean),
              })
            }
          />
          {/*
            Same provenance rule as create_node_types, and it has no
            alternative: a command is authored by a user and carries no
            __mount_id until the drain stamps one, so without a type filter the
            drain claims any node under this path whose status happens to read
            `queued` — a task, a draft, an agent's scratch node — and sends it.
          */}
          <p className="flex items-start gap-1.5 text-xs text-zinc-500 mt-1.5">
            <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
            Empty uses the command types RaisinDB ships. Set this when the outbox holds your own
            type — the drain has no other way to tell a command from ordinary content sitting under
            the same path.
          </p>
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
            <option value="local_wins">local_wins</option>
            <option value="error">error</option>
            <option value="resolver_function">resolver_function</option>
          </select>
          {/*
            Said at the point of choice, not in a doc page. `local_wins` is the
            one control in this editor whose wrong setting destroys data that
            cannot be recovered from here — everything else costs at worst a
            re-import.
          */}
          {value.conflict === 'local_wins' && (
            <p className="mt-1.5 text-xs text-amber-400">
              A refused push is re-sent with no concurrency check, OVERWRITING whatever changed at
              the provider. That change is not recoverable.
            </p>
          )}
          {value.conflict === 'remote_wins' && (
            <p className="mt-1.5 text-xs text-zinc-500">
              A refused push is abandoned and the next sync overwrites the local edit with the
              remote value. The edit is lost; the user can redo it.
            </p>
          )}
        </div>
      )}

      {/*
        Shown whenever the mount HAS a resolver, not only when the select says
        so: a path set here is on its own enough to select the resolver, and
        hiding the field behind the select would make a live resolver invisible.
      */}
      {mode !== 'off' && (value.conflict === 'resolver_function' || resolverFunction) && (
        <div>
          <label className={labelCls}>Resolver function</label>
          <input
            className={field}
            value={resolverFunction}
            placeholder="/resolvers/ms-graph-mail"
            onChange={(e) => onResolverChange(e.target.value)}
          />
          <p className="mt-1.5 text-xs text-zinc-500">
            Called on every refused push with the local node, the base etag and a two-sided field
            diff; it answers local_wins, remote_wins, merged or park. Anything unclear — a throw, an
            unrecognized answer — parks the edit rather than guessing. Unlike the adapter, this has
            no connector-level fallback.
          </p>
          {value.conflict === 'resolver_function' && !resolverFunction.trim() && (
            <p className="mt-1.5 text-xs text-red-400">
              Required: the engine refuses to drain this mount at all while the policy names a
              resolver that does not exist.
            </p>
          )}
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
        {state?.writeback_status && (
          <Row label="Outbound">
            <span className="text-amber-400 text-xs">
              {state.writeback_status === 'conflict'
                ? 'conflict — edits are parked awaiting a human'
                : state.writeback_status}
            </span>
          </Row>
        )}
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
