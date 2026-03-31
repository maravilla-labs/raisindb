import { useState } from 'react'
import { Layers, Zap, Code, ChevronRight, ChevronDown } from 'lucide-react'

interface QueryPlanViewProps {
  plan: string
}

export function QueryPlanView({ plan }: QueryPlanViewProps) {
  // Parse the plan into sections
  const sections = parsePlanSections(plan)

  const [expanded, setExpanded] = useState<Record<string, boolean>>({
    physical: true,
    optimized: sections.optimized !== null,
    logical: sections.logical !== null,
  })

  return (
    <div className="bg-white/5 backdrop-blur-md border border-white/10 rounded-xl p-6">
      <div className="flex items-center gap-2 mb-6">
        <Layers className="w-5 h-5 text-primary-400" />
        <h3 className="text-lg font-semibold text-white">Query Execution Plan</h3>
      </div>

      {/* Physical Plan (always shown) */}
      {sections.physical && (
        <div className="mb-4">
          <button
            onClick={() => setExpanded(prev => ({ ...prev, physical: !prev.physical }))}
            className="flex items-center gap-2 text-sm font-medium text-white mb-3 hover:text-primary-400 transition-colors"
          >
            {expanded.physical ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
            <Zap className="w-4 h-4 text-yellow-400" />
            <span>Physical Execution Plan</span>
          </button>
          {expanded.physical && (
            <pre className="bg-black/40 rounded-lg p-4 text-sm text-zinc-300 font-mono overflow-x-auto border border-white/10 whitespace-pre">
              {sections.physical}
            </pre>
          )}
        </div>
      )}

      {/* Optimized Logical Plan (if verbose) */}
      {sections.optimized && (
        <div className="mb-4">
          <button
            onClick={() => setExpanded(prev => ({ ...prev, optimized: !prev.optimized }))}
            className="flex items-center gap-2 text-sm font-medium text-white mb-3 hover:text-primary-400 transition-colors"
          >
            {expanded.optimized ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
            <Zap className="w-4 h-4 text-green-400" />
            <span>Optimized Logical Plan</span>
          </button>
          {expanded.optimized && (
            <pre className="bg-black/40 rounded-lg p-4 text-sm text-zinc-300 font-mono overflow-x-auto border border-white/10 whitespace-pre">
              {sections.optimized}
            </pre>
          )}
        </div>
      )}

      {/* Logical Plan (if verbose) */}
      {sections.logical && (
        <div className="mb-4">
          <button
            onClick={() => setExpanded(prev => ({ ...prev, logical: !prev.logical }))}
            className="flex items-center gap-2 text-sm font-medium text-white mb-3 hover:text-primary-400 transition-colors"
          >
            {expanded.logical ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
            <Code className="w-4 h-4 text-blue-400" />
            <span>Logical Plan (Original)</span>
          </button>
          {expanded.logical && (
            <pre className="bg-black/40 rounded-lg p-4 text-sm text-zinc-300 font-mono overflow-x-auto border border-white/10 whitespace-pre">
              {sections.logical}
            </pre>
          )}
        </div>
      )}
    </div>
  )
}

/**
 * Parse the explain plan text into sections
 */
function parsePlanSections(plan: string): {
  logical: string | null
  optimized: string | null
  physical: string
} {
  const logicalMatch = plan.match(/=== Logical Plan ===\n([\s\S]*?)(?:\n\n===|$)/)
  const optimizedMatch = plan.match(/=== Optimized Logical Plan ===\n([\s\S]*?)(?:\n\n===|$)/)
  const physicalMatch = plan.match(/=== Physical Execution Plan ===\n([\s\S]*)$/)

  return {
    logical: logicalMatch ? logicalMatch[1].trim() : null,
    optimized: optimizedMatch ? optimizedMatch[1].trim() : null,
    physical: physicalMatch ? physicalMatch[1].trim() : plan.trim(),
  }
}
