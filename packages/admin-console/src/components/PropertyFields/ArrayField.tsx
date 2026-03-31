import { Plus, X, List } from 'lucide-react'
import { getSchemaType, getSchemaLabel } from '../../utils/propertySchema'

interface ArrayFieldProps {
  name: string
  label: string
  value: any[] | undefined
  error?: string
  required?: boolean
  itemType?: any
  onChange: (value: any[]) => void
}

export default function ArrayField({
  name: _name,
  label,
  value,
  error,
  required,
  itemType,
  onChange
}: ArrayFieldProps) {
  const items = value || []

  const handleAdd = () => {
    const itemSchemaType = getSchemaType(itemType)
    let newItem: any = ''
    switch (itemSchemaType) {
      case 'boolean':
        newItem = false
        break
      case 'number':
      case 'integer':
        newItem = 0
        break
      case 'array':
        newItem = []
        break
      case 'object':
        newItem = {}
        break
      default:
        newItem = ''
    }
    onChange([...items, newItem])
  }

  const handleRemove = (index: number) => {
    onChange(items.filter((_, i) => i !== index))
  }

  const handleItemChange = (index: number, newValue: any) => {
    const updated = [...items]
    updated[index] = newValue
    onChange(updated)
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-2">
        <label className="flex items-center gap-2 text-sm font-medium text-zinc-300">
          <List className="w-4 h-4 text-yellow-400" />
          {label}
          {required && <span className="text-red-400 ml-1">*</span>}
        </label>
        <button
          type="button"
          onClick={handleAdd}
          className="flex items-center gap-1 px-2 py-1 bg-green-500/20 hover:bg-green-500/30 text-green-400 rounded text-sm transition-colors"
        >
          <Plus className="w-3 h-3" />
          Add Item
        </button>
      </div>

      <div className="space-y-2">
        {items.map((item, index) => (
          <div key={index} className="flex items-start gap-2">
            <span className="text-sm text-zinc-500 mt-2 min-w-[2rem]">{index + 1}.</span>
            <input
              type="text"
              value={typeof item === 'object' ? JSON.stringify(item) : String(item ?? '')}
              placeholder={
                itemType ? getSchemaLabel(`Item ${index + 1}`, itemType) : undefined
              }
              onChange={(e) => {
                const text = e.target.value
                const schemaType = getSchemaType(itemType)
                if (schemaType === 'number' || schemaType === 'integer') {
                  const parsed = Number(text)
                  handleItemChange(index, Number.isNaN(parsed) ? text : parsed)
                  return
                }
                if (schemaType === 'boolean') {
                  handleItemChange(index, text === 'true')
                  return
                }
                if (schemaType === 'object' || schemaType === 'array') {
                  try {
                    const val = JSON.parse(text)
                    handleItemChange(index, val)
                    return
                  } catch {
                    // fall through to plain text
                  }
                }
                handleItemChange(index, text)
              }}
              className="flex-1 px-3 py-1.5 bg-white/10 border border-white/20 rounded-lg text-white text-sm focus:outline-none focus:ring-2 focus:ring-primary-500"
            />
            <button
              type="button"
              onClick={() => handleRemove(index)}
              className="p-1.5 hover:bg-red-500/20 text-red-400 rounded transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        ))}
        {items.length === 0 && (
          <div className="text-sm text-zinc-500 py-2">
            No items yet. Click "Add Item" to start.
          </div>
        )}
      </div>

      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}
