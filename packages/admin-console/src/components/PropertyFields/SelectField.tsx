import { ChevronDown } from 'lucide-react'

interface SelectFieldProps {
  name: string
  label: string
  value: string | undefined
  options: string[] | { value: string; label: string }[]
  error?: string
  required?: boolean
  placeholder?: string
  onChange: (value: string) => void
}

export default function SelectField({
  name,
  label,
  value,
  options,
  error,
  required,
  placeholder = 'Select...',
  onChange
}: SelectFieldProps) {
  const normalizedOptions = options.map(opt =>
    typeof opt === 'string' ? { value: opt, label: opt } : opt
  )

  return (
    <div>
      <label htmlFor={name} className="block text-sm font-medium text-zinc-300 mb-2">
        <ChevronDown className="w-4 h-4 inline-block mr-1 text-secondary-400" />
        {label}
        {required && <span className="text-red-400 ml-1">*</span>}
      </label>
      <select
        id={name}
        value={value || ''}
        onChange={(e) => onChange(e.target.value)}
        className={`w-full px-4 py-2 bg-white/10 border rounded-lg text-white focus:outline-none focus:ring-2 appearance-none ${
          error
            ? 'border-red-500/50 focus:ring-red-500'
            : 'border-white/20 focus:ring-primary-500'
        }`}
      >
        <option value="" className="bg-zinc-900">{placeholder}</option>
        {normalizedOptions.map((opt) => (
          <option key={opt.value} value={opt.value} className="bg-zinc-900">
            {opt.label}
          </option>
        ))}
      </select>
      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}