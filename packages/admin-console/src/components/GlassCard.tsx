import { ReactNode } from 'react'

interface GlassCardProps {
  children: ReactNode
  className?: string
  hover?: boolean
  onClick?: () => void
}

export default function GlassCard({ children, className = '', hover = false, onClick }: GlassCardProps) {
  return (
    <div
      onClick={onClick}
      className={`glass rounded-xl p-6 transition-all duration-300 ${
        hover ? 'hover:scale-[1.02] hover:shadow-2xl hover:shadow-purple-500/20' : ''
      } ${className}`}
    >
      {children}
    </div>
  )
}
