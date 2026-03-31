import { useState } from 'react'
import {
  Code2,
  Columns,
  FileText,
  Minus,
  Plus,
  Equal,
} from 'lucide-react'
import type { FileDiff } from '../api/packages'

interface DiffViewerProps {
  diff: FileDiff
  className?: string
}

type ViewMode = 'unified' | 'split'

interface DiffLine {
  type: 'add' | 'remove' | 'context'
  content: string
  oldLineNum?: number
  newLineNum?: number
}

function parseDiffLine(line: string): DiffLine {
  if (line.startsWith('+') && !line.startsWith('+++')) {
    return { type: 'add', content: line.slice(1) }
  } else if (line.startsWith('-') && !line.startsWith('---')) {
    return { type: 'remove', content: line.slice(1) }
  } else if (line.startsWith(' ')) {
    return { type: 'context', content: line.slice(1) }
  }
  return { type: 'context', content: line }
}

function parseUnifiedDiff(diffText: string): DiffLine[] {
  const lines = diffText.split('\n')
  const result: DiffLine[] = []
  let oldLine = 1
  let newLine = 1

  for (const line of lines) {
    // Skip header lines
    if (line.startsWith('---') || line.startsWith('+++') || line.startsWith('@@')) {
      // Extract line numbers from @@ header
      const match = line.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/)
      if (match) {
        oldLine = parseInt(match[1], 10)
        newLine = parseInt(match[2], 10)
      }
      continue
    }

    const parsed = parseDiffLine(line)

    if (parsed.type === 'add') {
      parsed.newLineNum = newLine++
    } else if (parsed.type === 'remove') {
      parsed.oldLineNum = oldLine++
    } else {
      parsed.oldLineNum = oldLine++
      parsed.newLineNum = newLine++
    }

    result.push(parsed)
  }

  return result
}

export default function DiffViewer({ diff, className = '' }: DiffViewerProps) {
  const [viewMode, setViewMode] = useState<ViewMode>('unified')

  const diffLines = diff.unified_diff ? parseUnifiedDiff(diff.unified_diff) : []

  // For split view, separate lines into left (old) and right (new)
  const splitLines: Array<{ old?: DiffLine; new?: DiffLine }> = []
  if (viewMode === 'split') {
    let i = 0
    while (i < diffLines.length) {
      const line = diffLines[i]

      if (line.type === 'context') {
        splitLines.push({ old: line, new: line })
        i++
      } else if (line.type === 'remove') {
        // Check if next line is an add (replacement)
        if (i + 1 < diffLines.length && diffLines[i + 1].type === 'add') {
          splitLines.push({ old: line, new: diffLines[i + 1] })
          i += 2
        } else {
          splitLines.push({ old: line })
          i++
        }
      } else if (line.type === 'add') {
        splitLines.push({ new: line })
        i++
      } else {
        i++
      }
    }
  }

  function getLineClass(type: DiffLine['type']): string {
    switch (type) {
      case 'add':
        return 'bg-green-500/20 text-green-300'
      case 'remove':
        return 'bg-red-500/20 text-red-300'
      default:
        return 'text-zinc-400'
    }
  }

  function getLineIcon(type: DiffLine['type']) {
    switch (type) {
      case 'add':
        return <Plus className="w-3 h-3 text-green-400" />
      case 'remove':
        return <Minus className="w-3 h-3 text-red-400" />
      default:
        return <Equal className="w-3 h-3 text-zinc-600" />
    }
  }

  return (
    <div className={`bg-zinc-900 border border-white/10 rounded-lg overflow-hidden ${className}`}>
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-white/10 bg-black/20">
        <div className="flex items-center gap-2">
          <FileText className="w-4 h-4 text-zinc-400" />
          <span className="text-sm text-white font-medium">{diff.path}</span>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setViewMode('unified')}
            className={`flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors ${
              viewMode === 'unified'
                ? 'bg-primary-500/20 text-primary-400'
                : 'text-zinc-400 hover:text-white'
            }`}
          >
            <Code2 className="w-3 h-3" />
            Unified
          </button>
          <button
            onClick={() => setViewMode('split')}
            className={`flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors ${
              viewMode === 'split'
                ? 'bg-primary-500/20 text-primary-400'
                : 'text-zinc-400 hover:text-white'
            }`}
          >
            <Columns className="w-3 h-3" />
            Split
          </button>
        </div>
      </div>

      {/* Diff Content */}
      <div className="overflow-x-auto">
        {diffLines.length === 0 && !diff.local_content && !diff.server_content ? (
          <div className="p-8 text-center text-zinc-500">
            No diff available
          </div>
        ) : viewMode === 'unified' ? (
          /* Unified View */
          <table className="w-full text-xs font-mono">
            <tbody>
              {diffLines.map((line, idx) => (
                <tr key={idx} className={getLineClass(line.type)}>
                  <td className="w-10 px-2 py-0.5 text-right text-zinc-600 select-none border-r border-white/5">
                    {line.oldLineNum || ''}
                  </td>
                  <td className="w-10 px-2 py-0.5 text-right text-zinc-600 select-none border-r border-white/5">
                    {line.newLineNum || ''}
                  </td>
                  <td className="w-6 px-2 py-0.5 select-none">
                    {getLineIcon(line.type)}
                  </td>
                  <td className="px-2 py-0.5 whitespace-pre">{line.content}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          /* Split View */
          <div className="flex">
            {/* Left (Old) */}
            <div className="flex-1 border-r border-white/10">
              <div className="px-3 py-2 bg-red-500/10 text-red-400 text-xs font-medium border-b border-white/10">
                Local (Current)
              </div>
              <table className="w-full text-xs font-mono">
                <tbody>
                  {splitLines.map((pair, idx) => (
                    <tr
                      key={idx}
                      className={pair.old ? getLineClass(pair.old.type) : 'bg-zinc-800/50'}
                    >
                      <td className="w-10 px-2 py-0.5 text-right text-zinc-600 select-none border-r border-white/5">
                        {pair.old?.oldLineNum || pair.old?.newLineNum || ''}
                      </td>
                      <td className="px-2 py-0.5 whitespace-pre">
                        {pair.old?.content || ''}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            {/* Right (New) */}
            <div className="flex-1">
              <div className="px-3 py-2 bg-green-500/10 text-green-400 text-xs font-medium border-b border-white/10">
                Server (Original)
              </div>
              <table className="w-full text-xs font-mono">
                <tbody>
                  {splitLines.map((pair, idx) => (
                    <tr
                      key={idx}
                      className={pair.new ? getLineClass(pair.new.type) : 'bg-zinc-800/50'}
                    >
                      <td className="w-10 px-2 py-0.5 text-right text-zinc-600 select-none border-r border-white/5">
                        {pair.new?.newLineNum || ''}
                      </td>
                      <td className="px-2 py-0.5 whitespace-pre">
                        {pair.new?.content || ''}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>

      {/* Hash Info */}
      {(diff.local_hash || diff.server_hash) && (
        <div className="px-4 py-2 border-t border-white/10 bg-black/20 text-xs text-zinc-500 flex gap-6">
          {diff.local_hash && (
            <span>Local: {diff.local_hash.slice(0, 16)}...</span>
          )}
          {diff.server_hash && (
            <span>Server: {diff.server_hash.slice(0, 16)}...</span>
          )}
        </div>
      )}
    </div>
  )
}
