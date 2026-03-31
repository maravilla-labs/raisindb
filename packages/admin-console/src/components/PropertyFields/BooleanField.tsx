import { ToggleLeft } from 'lucide-react'

interface BooleanFieldProps {
  name: string
  label: string
  value: boolean | undefined
  error?: string
  required?: boolean
  onChange: (value: boolean) => void
}

export default function BooleanField({
  name,
  label,
  value,
  error,
  required,
  onChange
}: BooleanFieldProps) {
  return (
    <div>
      <label htmlFor={name} className="block text-sm font-medium text-zinc-300 mb-2">
        <ToggleLeft className="w-4 h-4 inline-block mr-1 text-primary-400" />
        {label}
        {required && <span className="text-red-400 ml-1">*</span>}
      </label>
      <button
        id={name}
        type="button"
        onClick={() => onChange(!value)}
        className={`relative inline-flex h-8 w-14 items-center rounded-full transition-colors ${
          value ? 'bg-primary-500' : 'bg-white/20'
        } ${error ? 'ring-2 ring-red-500' : ''}`}
      >
        <span className="sr-only">Toggle {label}</span>
        <span
          className={`inline-block h-6 w-6 transform rounded-full bg-white transition-transform ${
            value ? 'translate-x-7' : 'translate-x-1'
          }`}
        />
      </button>
      <span className="ml-3 text-sm text-zinc-300">{value ? 'Yes' : 'No'}</span>
      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}