import { LucideIcon } from 'lucide-react'
import { Loader2 } from 'lucide-react'

interface ActionButtonProps {
  onClick: () => void
  loading?: boolean
  disabled?: boolean
  icon?: LucideIcon
  children: React.ReactNode
  variant?: 'primary' | 'secondary' | 'danger'
  className?: string
}

export default function ActionButton({
  onClick,
  loading = false,
  disabled = false,
  icon: Icon,
  children,
  variant = 'primary',
  className = '',
}: ActionButtonProps) {
  const variantClasses = {
    primary: 'bg-purple-500 hover:bg-purple-600 disabled:bg-purple-500/50',
    secondary: 'bg-blue-500 hover:bg-blue-600 disabled:bg-blue-500/50',
    danger: 'bg-red-500 hover:bg-red-600 disabled:bg-red-500/50',
  }

  return (
    <button
      onClick={onClick}
      disabled={disabled || loading}
      className={`
        flex items-center gap-2 px-4 py-2 rounded-lg text-white font-medium
        transition-all duration-200
        disabled:opacity-50 disabled:cursor-not-allowed
        ${variantClasses[variant]}
        ${className}
      `}
    >
      {loading ? (
        <Loader2 className="w-5 h-5 animate-spin" />
      ) : (
        Icon && <Icon className="w-5 h-5" />
      )}
      {children}
    </button>
  )
}
