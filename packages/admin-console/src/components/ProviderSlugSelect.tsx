// SPDX-License-Identifier: BSL-1.1

import { useEffect, useState } from 'react'
import type { ProviderConfigResponse, ProviderSlug } from '../api/ai'
import { providerLabel, usableProviders } from '../utils/aiProviders'

/**
 * Picks one of the tenant's configured providers BY SLUG.
 *
 * Every consumer of this control stores a slug: an agent's `provider`, a
 * compaction override, `ai_provider_ref`. Offering kinds instead would make the
 * choice ambiguous the moment a tenant configures two gateways of one kind, and
 * would hide a self-named one (`marvel`) entirely.
 *
 * A stored slug that no longer resolves is kept as an option rather than
 * silently snapping to the first entry — that would repoint the agent at a
 * different provider on the next save without anyone asking.
 */
interface ProviderSlugSelectProps {
  id?: string
  /** The tenant's full provider list, as returned by GET /ai/config. */
  providers: ProviderConfigResponse[]
  value: ProviderSlug
  onChange: (slug: ProviderSlug) => void
  required?: boolean
  /** Text for the "nothing selected" option. Omit to force a choice. */
  emptyLabel?: string
  className?: string
}

/** Small logo for the selected entry, when it ships one. */
function SelectedIcon({ entry }: { entry: ProviderConfigResponse }) {
  const [failed, setFailed] = useState(false)
  useEffect(() => setFailed(false), [entry.icon_url])

  if (!entry.icon_url || failed) return null
  return (
    <img
      src={entry.icon_url}
      alt=""
      className="w-5 h-5 rounded object-contain flex-shrink-0"
      onError={() => setFailed(true)}
    />
  )
}

export default function ProviderSlugSelect({
  id,
  providers,
  value,
  onChange,
  required,
  emptyLabel,
  className = 'w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20',
}: ProviderSlugSelectProps) {
  const options = usableProviders(providers)
  const selected = providers.find((p) => p.slug === value)
  const unknownSelection = value !== '' && !options.some((p) => p.slug === value)

  return (
    <div className="flex items-center gap-2">
      {selected && <SelectedIcon entry={selected} />}
      <select
        id={id}
        required={required}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={className}
      >
        {emptyLabel !== undefined && <option value="">{emptyLabel}</option>}
        {options.map((p) => (
          <option key={p.slug} value={p.slug}>
            {providerLabel(p)}
          </option>
        ))}
        {unknownSelection && (
          <option value={value}>{value} (not configured)</option>
        )}
        {options.length === 0 && emptyLabel === undefined && !unknownSelection && (
          <option value="" disabled>
            No providers configured — add one in Admin Console &gt; AI Settings
          </option>
        )}
      </select>
    </div>
  )
}
