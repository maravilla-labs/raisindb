interface TimeTravelBannerProps {
  revision: string  // HLC format: "timestamp-counter"
  timestamp?: string
  actor?: string
  onExitTimeTravel: () => void
}

function formatDistanceToNow(date: Date): string {
  const seconds = Math.floor((new Date().getTime() - date.getTime()) / 1000)
  
  if (seconds < 60) return 'just now'
  if (seconds < 3600) return `${Math.floor(seconds / 60)} minutes ago`
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} hours ago`
  if (seconds < 2592000) return `${Math.floor(seconds / 86400)} days ago`
  return `${Math.floor(seconds / 2592000)} months ago`
}

export default function TimeTravelBanner({
  revision,
  timestamp,
  actor,
  onExitTimeTravel,
}: TimeTravelBannerProps) {
  return (
    <div className="bg-gradient-to-r from-amber-600 to-orange-600 border-b border-amber-500/50 px-6 py-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <svg className="w-5 h-5 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <div>
            <span className="text-white font-semibold">
              Viewing revision #{revision} (Read-Only)
            </span>
            {timestamp && (
              <span className="ml-2 text-white/80 text-sm">
                · {formatDistanceToNow(new Date(timestamp))}
              </span>
            )}
            {actor && (
              <span className="ml-2 text-white/80 text-sm">
                by {actor}
              </span>
            )}
          </div>
        </div>
        
        <button
          onClick={onExitTimeTravel}
          className="px-4 py-2 rounded bg-white/20 hover:bg-white/30 text-white font-medium transition-colors"
        >
          Return to HEAD
        </button>
      </div>
    </div>
  )
}
