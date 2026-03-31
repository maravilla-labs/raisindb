import { Hash } from 'lucide-react'

interface NumberFieldProps {
  name: string
  label: string
  value: number | undefined
  error?: string
  required?: boolean
  min?: number
  max?: number
  step?: number
  onChange: (value: number | undefined) => void
}

export default function NumberField({
  name,
  label,
  value,
  error,
  required,
  min,
  max,
  step,
  onChange
}: NumberFieldProps) {
  return (
    <div>
      <label htmlFor={name} className="block text-sm font-medium text-zinc-300 mb-2">
        <Hash className="w-4 h-4 inline-block mr-1 text-green-400" />
        {label}
        {required && <span className="text-red-400 ml-1">*</span>}
      </label>
      <input
        id={name}
        type="number"
        value={value ?? ''}
        onChange={(e) => {
          const val = e.target.value
          onChange(val === '' ? undefined : Number(val))
        }}
        min={min}
        max={max}
        step={step}
        className={`w-full px-4 py-2 bg-white/10 border rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 ${
          error
            ? 'border-red-500/50 focus:ring-red-500'
            : 'border-white/20 focus:ring-primary-500'
        }`}
      />
      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
      {(min !== undefined || max !== undefined) && (
        <p className="mt-1 text-xs text-zinc-500">
          {min !== undefined && `Min: ${min}`}
          {min !== undefined && max !== undefined && ' • '}
          {max !== undefined && `Max: ${max}`}
        </p>
      )}
    </div>
  )
}