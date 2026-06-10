/**
 * Priority badge
 *
 * Chip for task priority 1-5 (5 highest). Reusable outside the inbox page.
 */

interface PriorityBadgeProps {
  /** 1-5, 5 highest */
  priority?: number
}

const PRIORITY_CONFIG: Record<number, { label: string; color: string; bg: string }> = {
  5: { label: 'Critical', color: 'text-red-400', bg: 'bg-red-500/10' },
  4: { label: 'High', color: 'text-orange-400', bg: 'bg-orange-500/10' },
  3: { label: 'Medium', color: 'text-yellow-400', bg: 'bg-yellow-500/10' },
  2: { label: 'Low', color: 'text-zinc-400', bg: 'bg-zinc-500/10' },
  1: { label: 'Lowest', color: 'text-zinc-500', bg: 'bg-zinc-500/10' },
}

export default function PriorityBadge({ priority }: PriorityBadgeProps) {
  if (!priority) return null

  const clamped = Math.min(5, Math.max(1, Math.round(priority)))
  const config = PRIORITY_CONFIG[clamped]

  return (
    <span
      className={`inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium ${config.bg} ${config.color}`}
      title={`Priority ${clamped} of 5`}
    >
      P{clamped}
      <span className="hidden sm:inline">· {config.label}</span>
    </span>
  )
}
