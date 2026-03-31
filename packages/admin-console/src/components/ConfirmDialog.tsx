import { createPortal } from 'react-dom'
import { X, AlertTriangle } from 'lucide-react'

interface ConfirmDialogProps {
  open: boolean
  title: string
  message: string
  confirmText?: string
  cancelText?: string
  variant?: 'danger' | 'warning' | 'info'
  onConfirm: () => void
  onCancel: () => void
  children?: React.ReactNode
}

export default function ConfirmDialog({
  open,
  title,
  message,
  confirmText = 'Confirm',
  cancelText = 'Cancel',
  variant = 'info',
  onConfirm,
  onCancel,
  children,
}: ConfirmDialogProps) {
  if (!open) return null
  const variantColors = {
    danger: 'bg-red-500 hover:bg-red-600',
    warning: 'bg-yellow-500 hover:bg-yellow-600',
    info: 'bg-purple-500 hover:bg-purple-600',
  }

  const iconColors = {
    danger: 'text-red-400',
    warning: 'text-yellow-400',
    info: 'text-purple-400',
  }

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50 overscroll-none">
      <div className="glass-dark rounded-xl max-w-md w-full p-6 animate-slide-in overscroll-contain">
        <div className="flex justify-between items-start mb-4">
          <div className="flex items-center gap-3">
            <AlertTriangle className={`w-6 h-6 ${iconColors[variant]}`} />
            <h2 className="text-xl font-bold text-white">{title}</h2>
          </div>
          <button
            onClick={onCancel}
            className="p-1 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-5 h-5 text-gray-400" />
          </button>
        </div>

        <p className="text-gray-300 mb-4 whitespace-pre-wrap">{message}</p>

        {children && <div className="mb-6">{children}</div>}

        <div className="flex gap-3 justify-end">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
          >
            {cancelText}
          </button>
          <button
            onClick={onConfirm}
            className={`px-4 py-2 text-white rounded-lg transition-colors ${variantColors[variant]}`}
          >
            {confirmText}
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}
