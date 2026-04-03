import { MapPin } from 'lucide-react'

interface LocationValue {
  lat: number
  lng: number
}

interface LocationFieldProps {
  name: string
  label: string
  value: LocationValue | undefined
  error?: string
  required?: boolean
  onChange: (value: LocationValue | undefined) => void
}

export default function LocationField({
  name,
  label,
  value,
  error,
  required,
  onChange,
}: LocationFieldProps) {
  const lat = value?.lat ?? ''
  const lng = value?.lng ?? ''

  const handleChange = (field: 'lat' | 'lng', raw: string) => {
    if (raw === '' && ((field === 'lat' && lng === '') || (field === 'lng' && lat === ''))) {
      onChange(undefined)
      return
    }
    const num = raw === '' ? 0 : Number(raw)
    if (field === 'lat') {
      onChange({ lat: num, lng: typeof lng === 'number' ? lng : Number(lng) || 0 })
    } else {
      onChange({ lat: typeof lat === 'number' ? lat : Number(lat) || 0, lng: num })
    }
  }

  const inputClass = `w-full px-4 py-2 bg-white/10 border rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 ${
    error
      ? 'border-red-500/50 focus:ring-red-500'
      : 'border-white/20 focus:ring-primary-500'
  }`

  return (
    <div>
      <label className="block text-sm font-medium text-zinc-300 mb-2">
        <MapPin className="w-4 h-4 inline-block mr-1 text-pink-400" />
        {label}
        {required && <span className="text-red-400 ml-1">*</span>}
      </label>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label htmlFor={`${name}-lat`} className="block text-xs text-zinc-500 mb-1">
            Latitude
          </label>
          <input
            id={`${name}-lat`}
            type="number"
            value={lat}
            onChange={(e) => handleChange('lat', e.target.value)}
            min={-90}
            max={90}
            step="any"
            placeholder="-90 to 90"
            className={inputClass}
          />
        </div>
        <div>
          <label htmlFor={`${name}-lng`} className="block text-xs text-zinc-500 mb-1">
            Longitude
          </label>
          <input
            id={`${name}-lng`}
            type="number"
            value={lng}
            onChange={(e) => handleChange('lng', e.target.value)}
            min={-180}
            max={180}
            step="any"
            placeholder="-180 to 180"
            className={inputClass}
          />
        </div>
      </div>
      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}
