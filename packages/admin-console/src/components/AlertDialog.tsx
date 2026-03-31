import { createPortal } from 'react-dom'
import { X, CheckCircle, XCircle, AlertTriangle, Info } from 'lucide-react'

interface AlertDialogProps {
  open: boolean
  title: string
  message: string
  variant?: 'success' | 'error' | 'warning' | 'info'
  confirmText?: string
  onClose: () => void
}

export default function AlertDialog({
  open,
  title,
  message,
  variant = 'info',
  confirmText = 'OK',
  onClose,
}: AlertDialogProps) {
  if (!open) return null

  const variantColors = {
    success: 'bg-green-500 hover:bg-green-600',
    error: 'bg-red-500 hover:bg-red-600',
    warning: 'bg-yellow-500 hover:bg-yellow-600',
    info: 'bg-purple-500 hover:bg-purple-600',
  }

  const iconColors = {
    success: 'text-green-400',
    error: 'text-red-400',
    warning: 'text-yellow-400',
    info: 'text-purple-400',
  }

  const icons = {
    success: CheckCircle,
    error: XCircle,
    warning: AlertTriangle,
    info: Info,
  }

  const Icon = icons[variant]

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50 overscroll-none">
      <div className="glass-dark rounded-xl max-w-md w-full p-6 animate-slide-in overscroll-contain">
        <div className="flex justify-between items-start mb-4">
          <div className="flex items-center gap-3">
            <Icon className={`w-6 h-6 ${iconColors[variant]}`} />
            <h2 className="text-xl font-bold text-white">{title}</h2>
          </div>
          <button
            onClick={onClose}
            className="p-1 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-5 h-5 text-gray-400" />
          </button>
        </div>

        <p className="text-gray-300 mb-6 whitespace-pre-wrap">{message}</p>

        <div className="flex justify-end">
          <button
            onClick={onClose}
            className={`px-6 py-2 text-white rounded-lg transition-colors ${variantColors[variant]}`}
          >
            {confirmText}
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}
