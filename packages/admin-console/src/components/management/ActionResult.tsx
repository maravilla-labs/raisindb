import { CheckCircle, AlertCircle, Info, XCircle } from 'lucide-react'

interface ActionResultProps {
  type: 'success' | 'error' | 'info' | 'warning'
  title?: string
  message?: string
  details?: React.ReactNode
  onDismiss?: () => void
  className?: string
}

export default function ActionResult({
  type,
  title,
  message,
  details,
  onDismiss,
  className = '',
}: ActionResultProps) {
  const config = {
    success: {
      icon: CheckCircle,
      bgColor: 'bg-green-500/10',
      borderColor: 'border-green-500/20',
      textColor: 'text-green-300',
      iconColor: 'text-green-400',
    },
    error: {
      icon: XCircle,
      bgColor: 'bg-red-500/10',
      borderColor: 'border-red-500/20',
      textColor: 'text-red-300',
      iconColor: 'text-red-400',
    },
    warning: {
      icon: AlertCircle,
      bgColor: 'bg-yellow-500/10',
      borderColor: 'border-yellow-500/20',
      textColor: 'text-yellow-300',
      iconColor: 'text-yellow-400',
    },
    info: {
      icon: Info,
      bgColor: 'bg-blue-500/10',
      borderColor: 'border-blue-500/20',
      textColor: 'text-blue-300',
      iconColor: 'text-blue-400',
    },
  }

  const { icon: Icon, bgColor, borderColor, textColor, iconColor } = config[type]

  return (
    <div className={`${bgColor} border ${borderColor} rounded-lg p-4 ${className}`}>
      <div className="flex items-start gap-3">
        <div className="flex-shrink-0">
          <Icon className={`w-5 h-5 ${iconColor}`} />
        </div>
        <div className="flex-1 min-w-0">
          {title && <h4 className={`font-semibold ${textColor} mb-1`}>{title}</h4>}
          {message && <p className={`text-sm ${textColor}`}>{message}</p>}
          {details && <div className="mt-2">{details}</div>}
        </div>
        {onDismiss && (
          <button
            onClick={onDismiss}
            className={`flex-shrink-0 ${textColor} hover:opacity-75 transition-opacity`}
            aria-label="Dismiss"
          >
            <XCircle className="w-4 h-4" />
          </button>
        )}
      </div>
    </div>
  )
}
