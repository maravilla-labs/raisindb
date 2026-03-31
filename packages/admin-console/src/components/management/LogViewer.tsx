/**
 * LogViewer Component
 *
 * Reusable component for displaying function execution logs with level-based coloring.
 */

import { useRef, useEffect, useState } from 'react'
import { Trash2, Copy, Check } from 'lucide-react'

export interface LogEntry {
  level: 'debug' | 'info' | 'warn' | 'error'
  message: string
  timestamp: string
}

const LOG_LEVEL_COLORS: Record<LogEntry['level'], string> = {
  debug: 'text-gray-400',
  info: 'text-blue-400',
  warn: 'text-yellow-400',
  error: 'text-red-400',
}

const LOG_LEVEL_BG: Record<LogEntry['level'], string> = {
  debug: 'bg-gray-500/10',
  info: 'bg-blue-500/10',
  warn: 'bg-yellow-500/10',
  error: 'bg-red-500/10',
}

interface LogViewerProps {
  logs: LogEntry[]
  /** Whether to auto-scroll to bottom when new logs arrive */
  autoScroll?: boolean
  /** Callback to clear logs */
  onClear?: () => void
  /** Maximum height (CSS value) */
  maxHeight?: string
  /** Show message count header */
  showHeader?: boolean
  /** Compact mode for inline display */
  compact?: boolean
  /** Show copy to clipboard button */
  showCopyButton?: boolean
}

export default function LogViewer({
  logs,
  autoScroll = true,
  onClear,
  maxHeight = '200px',
  showHeader = true,
  compact = false,
  showCopyButton = false,
}: LogViewerProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const [copied, setCopied] = useState(false)

  // Auto-scroll to bottom when new logs arrive
  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [logs, autoScroll])

  const handleCopy = async () => {
    const text = logs.map(log =>
      `[${formatTimestamp(log.timestamp)}] [${log.level.toUpperCase()}] ${log.message}`
    ).join('\n')
    await navigator.clipboard.writeText(text)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  if (logs.length === 0) {
    return (
      <div className="text-zinc-500 text-center py-4 text-sm">
        No logs captured
      </div>
    )
  }

  return (
    <div className="flex flex-col relative">
      {showHeader && (
        <div className="flex items-center justify-between px-3 py-1 border-b border-white/10">
          <span className="text-xs text-zinc-400">{logs.length} log message{logs.length !== 1 ? 's' : ''}</span>
          <div className="flex items-center gap-1">
            {showCopyButton && (
              <button
                onClick={handleCopy}
                className="p-1 text-zinc-400 hover:text-white hover:bg-white/10 rounded"
                title="Copy logs"
              >
                {copied ? <Check className="w-3 h-3 text-green-400" /> : <Copy className="w-3 h-3" />}
              </button>
            )}
            {onClear && (
              <button
                onClick={onClear}
                className="p-1 text-zinc-400 hover:text-white hover:bg-white/10 rounded"
                title="Clear logs"
              >
                <Trash2 className="w-3 h-3" />
              </button>
            )}
          </div>
        </div>
      )}
      {/* Floating copy button when header is hidden */}
      {!showHeader && showCopyButton && (
        <button
          onClick={handleCopy}
          className="absolute top-1 right-1 p-1.5 text-zinc-400 hover:text-white hover:bg-white/20 rounded z-10"
          title="Copy logs"
        >
          {copied ? <Check className="w-3.5 h-3.5 text-green-400" /> : <Copy className="w-3.5 h-3.5" />}
        </button>
      )}
      <div
        ref={scrollRef}
        className={`overflow-auto font-mono text-xs ${compact ? 'p-1' : 'p-2'}`}
        style={{ maxHeight }}
      >
        {logs.map((log, idx) => (
          <div
            key={idx}
            className={`flex gap-2 ${compact ? 'py-0.5' : 'py-1'} ${LOG_LEVEL_BG[log.level]} rounded px-1 mb-0.5`}
          >
            <span className="text-zinc-500 flex-shrink-0">
              {formatTimestamp(log.timestamp)}
            </span>
            <span className={`flex-shrink-0 uppercase w-12 font-semibold ${LOG_LEVEL_COLORS[log.level]}`}>
              [{log.level}]
            </span>
            <span className="text-zinc-300 whitespace-pre-wrap break-all">
              {log.message}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}

function formatTimestamp(timestamp: string): string {
  try {
    return new Date(timestamp).toLocaleTimeString()
  } catch {
    return timestamp
  }
}

/**
 * Parse logs from various formats into LogEntry array
 */
export function parseLogs(logs: unknown): LogEntry[] {
  if (!logs) return []

  // Handle array of strings (simple logs)
  if (Array.isArray(logs)) {
    return logs.map((log, idx) => {
      if (typeof log === 'string') {
        return parseLogString(log, idx)
      }
      // Already a LogEntry object
      if (typeof log === 'object' && log !== null) {
        const entry = log as Record<string, unknown>
        return {
          level: (entry.level as LogEntry['level']) || 'info',
          message: String(entry.message || ''),
          timestamp: String(entry.timestamp || new Date().toISOString()),
        }
      }
      return {
        level: 'info' as const,
        message: String(log),
        timestamp: new Date().toISOString(),
      }
    })
  }

  return []
}

/**
 * Parse a single log string, detecting level prefixes
 */
function parseLogString(msg: string, _idx: number): LogEntry {
  const timestamp = new Date().toISOString()

  // Try to detect level from prefix
  if (msg.startsWith('[ERROR]') || msg.startsWith('[error]')) {
    return {
      level: 'error',
      message: msg.replace(/^\[ERROR\]|\[error\]/i, '').trim(),
      timestamp,
    }
  }
  if (msg.startsWith('[WARN]') || msg.startsWith('[warn]')) {
    return {
      level: 'warn',
      message: msg.replace(/^\[WARN\]|\[warn\]/i, '').trim(),
      timestamp,
    }
  }
  if (msg.startsWith('[DEBUG]') || msg.startsWith('[debug]')) {
    return {
      level: 'debug',
      message: msg.replace(/^\[DEBUG\]|\[debug\]/i, '').trim(),
      timestamp,
    }
  }
  if (msg.startsWith('[INFO]') || msg.startsWith('[info]')) {
    return {
      level: 'info',
      message: msg.replace(/^\[INFO\]|\[info\]/i, '').trim(),
      timestamp,
    }
  }

  return {
    level: 'info',
    message: msg,
    timestamp,
  }
}
