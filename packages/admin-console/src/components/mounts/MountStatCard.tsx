// SPDX-License-Identifier: BSL-1.1

import type { LucideIcon } from 'lucide-react'

interface MountStatCardProps {
  label: string
  value: number | string
  Icon: LucideIcon
  /** Semantic accent, applied only when the value is non-zero. */
  tone?: 'neutral' | 'good' | 'warn' | 'bad'
  hint?: string
}

const TONE: Record<string, { text: string; bg: string }> = {
  neutral: { text: 'text-zinc-300', bg: 'bg-white/5' },
  good: { text: 'text-green-400', bg: 'bg-green-500/10' },
  warn: { text: 'text-yellow-400', bg: 'bg-yellow-500/10' },
  bad: { text: 'text-red-400', bg: 'bg-red-500/10' },
}

/**
 * One tile in the run stat row.
 *
 * Deliberately not `management/MetricCard`: that one is a full GlassCard with a
 * 3xl number and a purple icon chip, sized for a dashboard of four. Six of them
 * in a row inside an already-carded panel reads as noise, and the purple is off
 * the brand accent. This is the same idea at row density.
 *
 * A zero stays neutral whatever the tone — colouring "0 failed" red would make
 * a clean run look alarming.
 */
export default function MountStatCard({
  label,
  value,
  Icon,
  tone = 'neutral',
  hint,
}: MountStatCardProps) {
  const isZero = value === 0 || value === '0'
  const t = TONE[isZero ? 'neutral' : tone] ?? TONE.neutral

  return (
    <div className="rounded-lg border border-white/10 bg-white/[0.02] px-3 py-2.5" title={hint}>
      <div className="flex items-center gap-1.5 mb-1">
        <span className={`inline-flex items-center justify-center w-5 h-5 rounded ${t.bg}`}>
          <Icon className={`w-3 h-3 ${t.text}`} />
        </span>
        <span className="text-[11px] uppercase tracking-wide text-zinc-500">{label}</span>
      </div>
      <div className={`text-xl font-semibold tabular-nums ${isZero ? 'text-zinc-500' : t.text}`}>
        {typeof value === 'number' ? value.toLocaleString() : value}
      </div>
    </div>
  )
}
