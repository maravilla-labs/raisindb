import { LucideIcon } from 'lucide-react'
import GlassCard from '../GlassCard'

interface MetricCardProps {
  title: string
  value: string | number
  icon: LucideIcon
  subtitle?: string
  trend?: {
    value: number
    label: string
  }
  className?: string
}

export default function MetricCard({
  title,
  value,
  icon: Icon,
  subtitle,
  trend,
  className = '',
}: MetricCardProps) {
  return (
    <GlassCard className={className}>
      <div className="flex items-start justify-between">
        <div className="flex-1">
          <p className="text-sm text-gray-400 mb-1">{title}</p>
          <p className="text-3xl font-bold text-white mb-1">{value}</p>
          {subtitle && <p className="text-xs text-gray-500">{subtitle}</p>}
          {trend && (
            <div className="mt-2 flex items-center gap-1">
              <span
                className={`text-xs font-medium ${
                  trend.value > 0 ? 'text-green-400' : trend.value < 0 ? 'text-red-400' : 'text-gray-400'
                }`}
              >
                {trend.value > 0 ? '+' : ''}
                {trend.value}%
              </span>
              <span className="text-xs text-gray-500">{trend.label}</span>
            </div>
          )}
        </div>
        <div className="p-3 bg-purple-500/20 rounded-lg">
          <Icon className="w-6 h-6 text-purple-400" />
        </div>
      </div>
    </GlassCard>
  )
}
