// SPDX-License-Identifier: BSL-1.1

import { useState } from 'react'
import { Copy, Check } from 'lucide-react'

interface CopyableUrlFieldProps {
  /** Field label shown above the value. */
  label: string
  /** The URL (or placeholder-bearing string) to display read-only + copy. */
  value: string
  /** Optional helper text rendered under the field. */
  helper?: React.ReactNode
  /**
   * When true, styles the field as a warning (e.g. the value carries a
   * `{base}` placeholder because `RAISINDB_BASE_URL` is unset).
   */
  warn?: boolean
}

/**
 * Read-only, single-line URL display with a copy-to-clipboard button.
 *
 * Used to surface the exact provider-side URLs a user must register — the OAuth
 * redirect URI (per connector) and the push notification URL (per mount). The
 * value is never editable here; it is derived from the server's setup-urls
 * endpoint (or constructed client-side) so the user copies a canonical string.
 */
export default function CopyableUrlField({ label, value, helper, warn }: CopyableUrlFieldProps) {
  const [copied, setCopied] = useState(false)

  async function copy() {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      // Clipboard API unavailable (e.g. insecure context) — no-op; the value is
      // still visible and selectable for a manual copy.
    }
  }

  return (
    <div>
      <label className="block text-white text-sm font-medium mb-1.5">{label}</label>
      <div className="flex items-stretch gap-2">
        <input
          readOnly
          value={value}
          onFocus={(e) => e.currentTarget.select()}
          className={`w-full px-3 py-2 bg-black/30 border rounded-lg text-xs font-mono focus:outline-none focus:ring-2 focus:ring-primary-500 ${
            warn ? 'border-yellow-500/40 text-yellow-200' : 'border-white/10 text-zinc-200'
          }`}
        />
        <button
          type="button"
          onClick={copy}
          className="flex items-center gap-1.5 px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg text-xs text-white transition-colors flex-shrink-0"
          title="Copy to clipboard"
        >
          {copied ? <Check className="w-3.5 h-3.5 text-green-400" /> : <Copy className="w-3.5 h-3.5" />}
          {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
      {helper && <p className="mt-1 text-[11px] text-zinc-500">{helper}</p>}
    </div>
  )
}
