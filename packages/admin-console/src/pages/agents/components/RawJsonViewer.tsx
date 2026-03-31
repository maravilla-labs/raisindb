import { useState } from 'react'
import { ChevronDown, ChevronRight, Copy, Check } from 'lucide-react'

interface RawJsonViewerProps {
  data: unknown
  title?: string
  defaultCollapsed?: boolean
}

export default function RawJsonViewer({ data, title, defaultCollapsed = false }: RawJsonViewerProps) {
  const json = JSON.stringify(data, null, 2)
  const isLarge = json.length > 2000
  const [collapsed, setCollapsed] = useState(defaultCollapsed || isLarge)
  const [copied, setCopied] = useState(false)

  const copyToClipboard = () => {
    navigator.clipboard.writeText(json).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }

  return (
    <div className="bg-zinc-900 border border-white/10 rounded-lg overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 bg-white/5 border-b border-white/10">
        <button
          onClick={() => setCollapsed(!collapsed)}
          className="flex items-center gap-2 text-sm text-zinc-300 hover:text-white"
        >
          {collapsed ? <ChevronRight className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
          {title || 'JSON'}
          {isLarge && <span className="text-xs text-zinc-500">({Math.round(json.length / 1024)}KB)</span>}
        </button>
        <button
          onClick={copyToClipboard}
          className="p-1.5 text-zinc-400 hover:text-white hover:bg-white/10 rounded transition-colors"
          title="Copy JSON"
        >
          {copied ? <Check className="w-4 h-4 text-green-400" /> : <Copy className="w-4 h-4" />}
        </button>
      </div>
      {!collapsed && (
        <pre className="p-4 text-xs text-zinc-300 font-mono overflow-x-auto max-h-[600px] overflow-y-auto">
          {json}
        </pre>
      )}
    </div>
  )
}
