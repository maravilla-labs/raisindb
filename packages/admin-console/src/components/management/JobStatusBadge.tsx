import { JobStatus, formatJobStatus, getJobStatusColor } from '../../api/management'

interface JobStatusBadgeProps {
  status: JobStatus
  className?: string
}

export default function JobStatusBadge({ status, className = '' }: JobStatusBadgeProps) {
  const statusText = formatJobStatus(status)
  const color = getJobStatusColor(status)

  const colorClasses = {
    blue: 'bg-blue-500/20 text-blue-300 border-blue-400/30',
    yellow: 'bg-yellow-500/20 text-yellow-300 border-yellow-400/30',
    green: 'bg-green-500/20 text-green-300 border-green-400/30',
    red: 'bg-red-500/20 text-red-300 border-red-400/30',
    gray: 'bg-gray-500/20 text-gray-300 border-gray-400/30',
  }

  return (
    <span
      className={`inline-flex items-center px-3 py-1 rounded-full text-xs font-medium border ${
        colorClasses[color as keyof typeof colorClasses]
      } ${className}`}
    >
      {statusText}
    </span>
  )
}
