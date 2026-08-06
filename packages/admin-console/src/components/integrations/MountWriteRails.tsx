// SPDX-License-Identifier: BSL-1.1

import { Info } from 'lucide-react'
import type { Capabilities, WriteConfig } from '../../api/integrations'

const field =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500'
const labelCls = 'block text-white text-sm font-medium mb-1.5'

interface Props {
  value: WriteConfig
  onChange: (patch: Partial<WriteConfig>) => void
  caps?: Capabilities
}

/**
 * The `mirror`-only half of the write configuration: what a local DELETE and a
 * local MOVE mean remotely, which node types may be created, and the
 * blast-radius rails that make the first two survivable.
 *
 * Split out of `MountWriteFieldset` rather than inlined because every control
 * here is `mirror`-only and the fieldset was already at the size where the
 * `state_only` path became hard to read — the same reason `MountWriteFieldset`
 * itself came out of `MountEditor`.
 */
export default function MountWriteRails({ value, onChange, caps }: Props) {
  // The adapter's recommendation, which the engine applies when the mount says
  // nothing. Shown as the placeholder so an operator can see what "unset"
  // actually resolves to instead of assuming it means "nothing happens".
  const adapterDelete = caps?.default_delete_policy || 'detach'
  const adapterMove = caps?.default_move_policy || 'detach'
  const deletePolicy = value.delete_policy || ''
  const createTypes = value.create_node_types || []

  // A `trash` the adapter cannot serve is REFUSED by the engine, not quietly
  // promoted to a purge — the single worst substitution available here. Saying
  // so at the point of choice beats a mount that resolves to Refused after save.
  const trashUnsupported = caps?.supports_trash !== true

  return (
    <>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <div>
          <label className={labelCls}>Local delete</label>
          <select
            className={field}
            value={deletePolicy}
            onChange={(e) =>
              onChange({ delete_policy: (e.target.value || undefined) as WriteConfig['delete_policy'] })
            }
          >
            <option value="">connector default ({adapterDelete})</option>
            <option value="detach">detach — push nothing</option>
            <option value="trash" disabled={trashUnsupported}>
              trash — recoverable{trashUnsupported ? ' (not offered by this connector)' : ''}
            </option>
            <option value="purge">purge — irreversible</option>
          </select>
          {deletePolicy === 'detach' && (
            <p className="mt-1.5 text-xs text-zinc-500">
              The node is deleted locally and nothing is pushed — so the next full reconcile
              RE-IMPORTS it. There is no per-mount suppression list; the node comes back.
            </p>
          )}
          {deletePolicy === 'purge' && (
            <p className="mt-1.5 text-xs text-amber-400">
              A local delete destroys the remote object permanently. Not recoverable from here or
              from the provider.
            </p>
          )}
          {deletePolicy === 'trash' && trashUnsupported && (
            <p className="mt-1.5 text-xs text-red-400">
              This connector does not declare a trash. The engine refuses the mount rather than
              silently purging — choose detach or purge.
            </p>
          )}
        </div>

        <div>
          <label className={labelCls}>Local move</label>
          <select
            className={field}
            value={value.move_policy || ''}
            onChange={(e) =>
              onChange({ move_policy: (e.target.value || undefined) as WriteConfig['move_policy'] })
            }
          >
            <option value="">connector default ({adapterMove})</option>
            <option value="push">push — move the remote object too</option>
            <option value="detach">detach — leave the remote where it is</option>
            <option value="reject">reject — withhold the whole update</option>
          </select>
          <p className="mt-1.5 text-xs text-zinc-500">
            {value.move_policy === 'reject'
              ? 'While a location field diverges the ENTIRE update is withheld — a half-applied node is indistinguishable at the provider from a successful push.'
              : 'A move is an update carrying the new location field; there is no separate move operation. The local move sticks either way.'}
          </p>
        </div>
      </div>

      <div>
        <label className={labelCls}>Create these node types remotely</label>
        <textarea
          className={field}
          rows={2}
          placeholder={'raisin:Event'}
          value={createTypes.join('\n')}
          onChange={(e) =>
            onChange({
              create_node_types: e.target.value
                .split(/[\n,]/)
                .map((s) => s.trim())
                .filter(Boolean),
            })
          }
        />
        {/*
          The provenance rule, stated where it is chosen. A node authored under
          the mount path has no __mount_id and no __external_id — that absence is
          exactly what makes it a create candidate — so the ownership check every
          other write path makes is unavailable, and this list is the only thing
          standing in for it.
        */}
        <p className="flex items-start gap-1.5 text-xs text-zinc-500 mt-1.5">
          <Info className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
          {createTypes.length === 0
            ? 'Empty (the default): nodes authored here are never uploaded. A locally-created node has no provenance marking it as this mount’s, so the node TYPE is the only ownership signal — and guessing wrong sends private content to a third party, which no later config change undoes.'
            : 'Only these types are uploaded. Everything else authored under the mount path stays local.'}
        </p>
        {createTypes.length > 0 && caps?.can_create !== true && (
          <p className="mt-1.5 text-xs text-red-400">
            This connector does not declare <code>can_create</code>, so the engine refuses the
            mount while this list is non-empty. Clear it to keep update and delete working.
          </p>
        )}
      </div>

      {/*
        The rails. A mis-scoped `DELETE FROM 'ws' WHERE path LIKE '/mail/%'` must
        not wipe a real mailbox, and the allowance is deliberately BOTH an
        absolute floor and a proportion: an absolute cap alone is wrong in both
        directions — far too low for a ten-node mount, far too high for a
        200k-message one.
      */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 pt-1 border-t border-white/5">
        <div>
          <label className={labelCls}>Delete floor / run</label>
          <input
            className={field}
            type="number"
            min={0}
            placeholder="10"
            value={value.max_deletes_per_run ?? ''}
            onChange={(e) =>
              onChange({
                max_deletes_per_run: e.target.value === '' ? undefined : Number(e.target.value),
              })
            }
          />
        </div>
        <div>
          <label className={labelCls}>Delete ratio</label>
          <input
            className={field}
            type="number"
            min={0}
            max={1}
            step={0.01}
            placeholder="0.05"
            value={value.max_delete_ratio ?? ''}
            onChange={(e) =>
              onChange({
                max_delete_ratio: e.target.value === '' ? undefined : Number(e.target.value),
              })
            }
          />
        </div>
        <div className="flex items-end pb-2">
          <label className="flex items-center gap-2 text-white text-sm">
            <input
              type="checkbox"
              className="w-4 h-4 rounded"
              checked={value.require_confirmation_on_bulk !== false}
              onChange={(e) => onChange({ require_confirmation_on_bulk: e.target.checked })}
            />
            <span>Confirm bulk deletes</span>
          </label>
        </div>
      </div>
      <p className="text-xs text-zinc-500">
        A drain is blocked when pending deletes exceed <em>both</em> the floor and the ratio of the
        mount’s size — whichever is larger wins, so neither a tiny nor a huge mount is governed by
        the wrong one. Bulk confirmation is separate and triggers on the ORIGINATING transaction’s
        width, however few deletes reach any one drain. A tripped rail parks the deletes and blocks
        outbound writes only; reading carries on.
      </p>
    </>
  )
}
