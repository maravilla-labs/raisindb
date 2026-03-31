import { Type } from 'lucide-react'

interface StringFieldProps {
  name: string
  label: string
  value: string | undefined
  error?: string
  required?: boolean
  multiline?: boolean
  placeholder?: string
  onChange: (value: string) => void
}

export default function StringField({
  name,
  label,
  value,
  error,
  required,
  multiline,
  placeholder,
  onChange
}: StringFieldProps) {
  return (
    <div>
      <label htmlFor={name} className="block text-sm font-medium text-zinc-300 mb-2">
        <Type className="w-4 h-4 inline-block mr-1 text-blue-400" />
        {label}
        {required && <span className="text-red-400 ml-1">*</span>}
      </label>
      {multiline ? (
        <textarea
          id={name}
          value={value || ''}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          rows={4}
          className={`w-full px-4 py-2 bg-white/10 border rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 ${
            error
              ? 'border-red-500/50 focus:ring-red-500'
              : 'border-white/20 focus:ring-primary-500'
          }`}
        />
      ) : (
        <input
          id={name}
          type="text"
          value={value || ''}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className={`w-full px-4 py-2 bg-white/10 border rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 ${
            error
              ? 'border-red-500/50 focus:ring-red-500'
              : 'border-white/20 focus:ring-primary-500'
          }`}
        />
      )}
      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}