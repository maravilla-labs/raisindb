import { createPortal } from 'react-dom'
import { CheckCircle, XCircle, AlertCircle, Info, X } from 'lucide-react'
import { useEffect, useState } from 'react'

export type ToastType = 'success' | 'error' | 'warning' | 'info'

export interface ToastMessage {
  id: string
  type: ToastType
  title: string
  message?: string
  duration?: number
}

interface ToastProps {
  toast: ToastMessage
  onClose: (id: string) => void
}

function Toast({ toast, onClose }: ToastProps) {
  const [isExiting, setIsExiting] = useState(false)

  useEffect(() => {
    const duration = toast.duration ?? 5000
    if (duration > 0) {
      const timer = setTimeout(() => {
        setIsExiting(true)
        setTimeout(() => onClose(toast.id), 300)
      }, duration)

      return () => clearTimeout(timer)
    }
  }, [toast.id, toast.duration, onClose])

  const handleClose = () => {
    setIsExiting(true)
    setTimeout(() => onClose(toast.id), 300)
  }

  const config = {
    success: {
      icon: CheckCircle,
      bgColor: 'bg-green-500/10',
      borderColor: 'border-green-500/30',
      iconColor: 'text-green-400',
    },
    error: {
      icon: XCircle,
      bgColor: 'bg-red-500/10',
      borderColor: 'border-red-500/30',
      iconColor: 'text-red-400',
    },
    warning: {
      icon: AlertCircle,
      bgColor: 'bg-yellow-500/10',
      borderColor: 'border-yellow-500/30',
      iconColor: 'text-yellow-400',
    },
    info: {
      icon: Info,
      bgColor: 'bg-blue-500/10',
      borderColor: 'border-blue-500/30',
      iconColor: 'text-blue-400',
    },
  }

  const { icon: Icon, bgColor, borderColor, iconColor } = config[toast.type]

  return (
    <div
      className={`
        glass rounded-lg p-4 border ${borderColor} ${bgColor}
        shadow-lg max-w-md w-full
        transition-all duration-300 ease-out
        ${isExiting ? 'opacity-0 translate-x-full' : 'opacity-100 translate-x-0'}
      `}
      role="alert"
      aria-live="assertive"
    >
      <div className="flex items-start gap-3">
        <Icon className={`w-5 h-5 ${iconColor} flex-shrink-0 mt-0.5`} />
        <div className="flex-1 min-w-0">
          <h3 className="text-white font-medium text-sm">{toast.title}</h3>
          {toast.message && (
            <p className="text-gray-300 text-sm mt-1">{toast.message}</p>
          )}
        </div>
        <button
          onClick={handleClose}
          className="flex-shrink-0 p-1 hover:bg-white/10 rounded transition-colors"
          aria-label="Close notification"
        >
          <X className="w-4 h-4 text-gray-400" />
        </button>
      </div>
    </div>
  )
}

interface ToastContainerProps {
  toasts: ToastMessage[]
  onClose: (id: string) => void
}

export function ToastContainer({ toasts, onClose }: ToastContainerProps) {
  if (toasts.length === 0) return null

  return createPortal(
    <div
      className="fixed top-4 right-4 z-[100] flex flex-col gap-3 pointer-events-none"
      aria-live="polite"
      aria-atomic="true"
    >
      <div className="pointer-events-auto space-y-3">
        {toasts.map((toast) => (
          <Toast key={toast.id} toast={toast} onClose={onClose} />
        ))}
      </div>
    </div>,
    document.body
  )
}

// Toast manager hook
export function useToast() {
  const [toasts, setToasts] = useState<ToastMessage[]>([])

  const showToast = (
    type: ToastType,
    title: string,
    message?: string,
    duration?: number
  ) => {
    const id = Math.random().toString(36).substring(2, 9)
    setToasts((prev) => [...prev, { id, type, title, message, duration }])
  }

  const closeToast = (id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id))
  }

  return {
    toasts,
    showToast,
    closeToast,
    success: (title: string, message?: string, duration?: number) =>
      showToast('success', title, message, duration),
    error: (title: string, message?: string, duration?: number) =>
      showToast('error', title, message, duration),
    warning: (title: string, message?: string, duration?: number) =>
      showToast('warning', title, message, duration),
    info: (title: string, message?: string, duration?: number) =>
      showToast('info', title, message, duration),
  }
}
