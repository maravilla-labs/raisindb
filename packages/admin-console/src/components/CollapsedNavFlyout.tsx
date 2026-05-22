import { useCallback, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Link } from 'react-router-dom'
import { ChevronRight, type LucideIcon } from 'lucide-react'

export interface FlyoutItem {
  to: string
  icon: LucideIcon
  label: string
  active: boolean
}

interface CollapsedNavFlyoutProps {
  /** When false, children are rendered as-is (expanded sidebar — no flyout). */
  enabled: boolean
  /** Section title shown at the top of the flyout. */
  label: string
  items: FlyoutItem[]
  /** Called when a flyout item is clicked (e.g. to close a mobile drawer). */
  onNavigate?: () => void
  /** The trigger — the collapsed group icon. */
  children: ReactNode
}

/**
 * Wraps a collapsed-sidebar group icon and reveals its sub-items in a floating
 * panel on hover. The panel is rendered through a portal to `document.body` so
 * it escapes the sidebar's `overflow-y-auto` clipping; it's positioned with
 * `fixed` coordinates derived from the trigger's bounding rect.
 */
export default function CollapsedNavFlyout({
  enabled,
  label,
  items,
  onNavigate,
  children,
}: CollapsedNavFlyoutProps) {
  const triggerRef = useRef<HTMLDivElement>(null)
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [open, setOpen] = useState(false)
  const [pos, setPos] = useState<{ top: number; left: number }>({ top: 0, left: 0 })

  const show = useCallback(() => {
    if (closeTimer.current) {
      clearTimeout(closeTimer.current)
      closeTimer.current = null
    }
    const rect = triggerRef.current?.getBoundingClientRect()
    if (rect) {
      // Anchor to the right of the icon; clamp so a long list stays on screen.
      const top = Math.min(rect.top, window.innerHeight - 16)
      setPos({ top, left: rect.right + 8 })
    }
    setOpen(true)
  }, [])

  const scheduleClose = useCallback(() => {
    if (closeTimer.current) clearTimeout(closeTimer.current)
    closeTimer.current = setTimeout(() => setOpen(false), 120)
  }, [])

  if (!enabled) return <>{children}</>

  return (
    <div ref={triggerRef} onMouseEnter={show} onMouseLeave={scheduleClose} className="relative mx-auto w-fit">
      {children}
      {/* Cue that this icon expands into a sub-menu (revealed on hover). */}
      <ChevronRight className="pointer-events-none absolute right-0 top-1/2 -translate-y-1/2 w-3 h-3 text-white/45" />
      {open &&
        createPortal(
          <div
            onMouseEnter={show}
            onMouseLeave={scheduleClose}
            style={{ position: 'fixed', top: pos.top, left: pos.left, zIndex: 60 }}
            className="min-w-48 max-h-[70vh] overflow-y-auto thin-scrollbar rounded-lg border border-white/10 bg-zinc-900/95 backdrop-blur-md shadow-2xl shadow-black/50 p-2"
          >
            <div className="px-2 py-1 text-xs font-semibold uppercase tracking-wide text-white/40 select-none">
              {label}
            </div>
            {items.map((item) => (
              <Link
                key={item.to}
                to={item.to}
                onClick={() => {
                  setOpen(false)
                  onNavigate?.()
                }}
                className={`flex items-center gap-2.5 px-2.5 py-1.5 rounded-md text-sm transition-colors ${
                  item.active
                    ? 'bg-primary-500/30 text-white'
                    : 'text-white/75 hover:bg-white/10 hover:text-white'
                }`}
              >
                <item.icon className="w-4 h-4 flex-shrink-0" />
                <span>{item.label}</span>
              </Link>
            ))}
          </div>,
          document.body,
        )}
    </div>
  )
}
