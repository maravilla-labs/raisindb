import { HealthStatus } from '../../api/management'
import { Activity, AlertTriangle, XCircle } from 'lucide-react'

interface HealthIndicatorProps {
  status: HealthStatus['status']
  showLabel?: boolean
  className?: string
}

export default function HealthIndicator({ status, showLabel = true, className = '' }: HealthIndicatorProps) {
  const config = {
    Healthy: {
      icon: Activity,
      color: 'text-green-400',
      bg: 'bg-green-500/20',
      label: 'Healthy',
    },
    Degraded: {
      icon: AlertTriangle,
      color: 'text-yellow-400',
      bg: 'bg-yellow-500/20',
      label: 'Degraded',
    },
    Critical: {
      icon: XCircle,
      color: 'text-red-400',
      bg: 'bg-red-500/20',
      label: 'Critical',
    },
  }

  const { icon: Icon, color: textColor, bg, label } = config[status]

  return (
    <div className={`inline-flex items-center gap-2 ${className}`}>
      <div className={`p-2 rounded-lg ${bg}`}>
        <Icon className={`w-5 h-5 ${textColor}`} />
      </div>
      {showLabel && <span className={`font-medium ${textColor}`}>{label}</span>}
    </div>
  )
}
