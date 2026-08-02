// SPDX-License-Identifier: BSL-1.1

import { Eye, Pencil, Zap, Webhook, Search, FolderTree, HelpCircle } from 'lucide-react'
import type { Capabilities } from '../../api/integrations'

interface CapabilityChipsProps {
  capabilities?: Capabilities
  /** When true, render a compact row (smaller text, no "unknown" helper copy). */
  compact?: boolean
  className?: string
}

interface ChipDef {
  key: keyof Capabilities
  label: string
  Icon: typeof Eye
  title: string
}

const CHIPS: ChipDef[] = [
  { key: 'can_read', label: 'Read', Icon: Eye, title: 'Can list and read remote items' },
  { key: 'can_write', label: 'Write', Icon: Pencil, title: 'Can create/update remote items' },
  { key: 'supports_changes', label: 'Delta', Icon: Zap, title: 'Has a real delta API (incremental sync)' },
  { key: 'supports_webhooks', label: 'Webhooks', Icon: Webhook, title: 'Supports push notifications' },
  { key: 'supports_search', label: 'Search', Icon: Search, title: 'Supports remote search' },
  {
    key: 'supports_browse',
    label: 'Browse',
    Icon: FolderTree,
    title: 'Can list remote containers, so mounts offer pickers instead of raw ids',
  },
]

/** True when no capability probe has ever populated the object. */
export function capabilitiesUnknown(capabilities?: Capabilities): boolean {
  if (!capabilities) return true
  return CHIPS.every((c) => capabilities[c.key] === undefined)
}

/**
 * Renders the connector's advertised capabilities as small chips
 * (Read / Write / Delta / Webhooks / Search / Browse). Chips are lit when the flag is
 * true and muted when false. When capabilities were never probed, shows a
 * neutral "unknown" chip prompting a connection test.
 */
export default function CapabilityChips({ capabilities, compact, className = '' }: CapabilityChipsProps) {
  if (capabilitiesUnknown(capabilities)) {
    return (
      <span
        title="Capabilities unknown — run Test connection to probe this connector."
        className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-zinc-500/15 text-zinc-400 border border-zinc-500/25 ${className}`}
      >
        <HelpCircle className="w-2.5 h-2.5" /> Capabilities unknown
      </span>
    )
  }

  const size = compact ? 'text-[10px]' : 'text-xs'
  return (
    <div className={`flex flex-wrap items-center gap-1 ${className}`}>
      {CHIPS.map(({ key, label, Icon, title }) => {
        const on = capabilities?.[key] === true
        return (
          <span
            key={key as string}
            title={title}
            className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full font-medium border ${size} ${
              on
                ? 'bg-sky-500/20 text-sky-300 border-sky-500/30'
                : 'bg-zinc-500/10 text-zinc-500 border-zinc-500/20 line-through'
            }`}
          >
            <Icon className="w-2.5 h-2.5" /> {label}
          </span>
        )
      })}
    </div>
  )
}
