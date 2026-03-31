import { Home, ChevronRight } from 'lucide-react'

interface BreadcrumbSegment {
  label: string
  path: string
}

interface BreadcrumbProps {
  segments: BreadcrumbSegment[]
  onNavigate: (path: string) => void
  className?: string
}

export default function Breadcrumb({ segments, onNavigate, className = '' }: BreadcrumbProps) {
  return (
    <nav className={`flex items-center gap-2 flex-wrap ${className}`}>
      {/* Home / Root */}
      <button
        type="button"
        onClick={() => onNavigate('/')}
        className="flex items-center gap-1 text-white/60 hover:text-white transition-colors group"
        title="Root"
      >
        <Home className="w-4 h-4" />
        <span className="text-sm hidden md:inline">Root</span>
      </button>

      {segments.map((segment, index) => {
        const isLast = index === segments.length - 1

        return (
          <div key={segment.path} className="flex items-center gap-2">
            <ChevronRight className="w-4 h-4 text-white/30" />

            {isLast ? (
              <span className="text-white text-sm font-medium truncate max-w-[200px] md:max-w-[300px]">
                {segment.label}
              </span>
            ) : (
              <button
                type="button"
                onClick={() => onNavigate(segment.path)}
                className="text-white/60 hover:text-white transition-colors text-sm truncate max-w-[150px] md:max-w-[200px]"
              >
                {segment.label}
              </button>
            )}
          </div>
        )
      })}
    </nav>
  )
}
