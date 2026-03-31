/**
 * Output Panel Component
 *
 * Bottom panel with tabs for Output logs, Problems, and Executions.
 */

import { useState, useRef, useEffect } from 'react'
import { Terminal, AlertTriangle, History, Trash2, Play, CheckCircle, XCircle, Clock, RefreshCw, Loader2, Lightbulb } from 'lucide-react'
import { useFunctionsContext } from '../../hooks'
import type { LogEntry, ExecutionRecord, ValidationProblem, ProblemSeverity } from '../../types'

type TabId = 'output' | 'problems' | 'executions'

const LOG_LEVEL_COLORS: Record<LogEntry['level'], string> = {
  debug: 'text-gray-400',
  info: 'text-blue-400',
  warn: 'text-yellow-400',
  error: 'text-red-400',
}

const EXECUTION_STATUS_ICONS: Record<ExecutionRecord['status'], React.ReactNode> = {
  pending: <Clock className="w-4 h-4 text-gray-400" />,
  running: <Play className="w-4 h-4 text-blue-400 animate-pulse" />,
  completed: <CheckCircle className="w-4 h-4 text-green-400" />,
  failed: <XCircle className="w-4 h-4 text-red-400" />,
}

function OutputTab() {
  const { logs, clearLogs } = useFunctionsContext()
  const scrollRef = useRef<HTMLDivElement>(null)

  // Auto-scroll to bottom when new logs arrive
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [logs])

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-3 py-1 border-b border-white/10">
        <span className="text-xs text-gray-400">{logs.length} messages</span>
        <button
          onClick={clearLogs}
          className="p-1 text-gray-400 hover:text-white hover:bg-white/10 rounded"
          title="Clear output"
        >
          <Trash2 className="w-3 h-3" />
        </button>
      </div>
      <div ref={scrollRef} className="flex-1 overflow-auto p-2 font-mono text-xs">
        {logs.length === 0 ? (
          <div className="text-gray-500 text-center py-4">No output yet</div>
        ) : (
          logs.map((log, idx) => (
            <div key={idx} className="flex gap-2 py-0.5">
              <span className="text-gray-500 flex-shrink-0">
                {new Date(log.timestamp).toLocaleTimeString()}
              </span>
              <span className={`flex-shrink-0 uppercase w-12 ${LOG_LEVEL_COLORS[log.level]}`}>
                [{log.level}]
              </span>
              <span className="text-gray-300 whitespace-pre-wrap break-all">
                {log.message}
              </span>
            </div>
          ))
        )}
      </div>
    </div>
  )
}

const PROBLEM_SEVERITY_CONFIG: Record<ProblemSeverity, { icon: React.ReactNode; color: string; bgColor: string }> = {
  error: {
    icon: <XCircle className="w-4 h-4" />,
    color: 'text-red-400',
    bgColor: 'bg-red-500/10',
  },
  warning: {
    icon: <AlertTriangle className="w-4 h-4" />,
    color: 'text-yellow-400',
    bgColor: 'bg-yellow-500/10',
  },
  suggestion: {
    icon: <Lightbulb className="w-4 h-4" />,
    color: 'text-blue-400',
    bgColor: 'bg-blue-500/10',
  },
}

function ProblemsTab() {
  const { problems, clearProblems } = useFunctionsContext()

  // Group problems by source
  const groupedProblems = problems.reduce<Record<string, ValidationProblem[]>>((acc, problem) => {
    const source = problem.source || 'Unknown'
    if (!acc[source]) {
      acc[source] = []
    }
    acc[source].push(problem)
    return acc
  }, {})

  // Count by severity
  const errorCount = problems.filter((p) => p.severity === 'error').length
  const warningCount = problems.filter((p) => p.severity === 'warning').length
  const suggestionCount = problems.filter((p) => p.severity === 'suggestion').length

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-3 py-1 border-b border-white/10">
        <div className="flex items-center gap-3 text-xs">
          {errorCount > 0 && (
            <span className="flex items-center gap-1 text-red-400">
              <XCircle className="w-3 h-3" />
              {errorCount}
            </span>
          )}
          {warningCount > 0 && (
            <span className="flex items-center gap-1 text-yellow-400">
              <AlertTriangle className="w-3 h-3" />
              {warningCount}
            </span>
          )}
          {suggestionCount > 0 && (
            <span className="flex items-center gap-1 text-blue-400">
              <Lightbulb className="w-3 h-3" />
              {suggestionCount}
            </span>
          )}
          {problems.length === 0 && (
            <span className="text-gray-400">No problems</span>
          )}
        </div>
        {problems.length > 0 && (
          <button
            onClick={clearProblems}
            className="p-1 text-gray-400 hover:text-white hover:bg-white/10 rounded"
            title="Clear problems"
          >
            <Trash2 className="w-3 h-3" />
          </button>
        )}
      </div>
      <div className="flex-1 overflow-auto">
        {problems.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-gray-400 text-sm">
            <CheckCircle className="w-8 h-8 mb-2 text-green-400/50" />
            No problems detected
          </div>
        ) : (
          <div className="divide-y divide-white/5">
            {Object.entries(groupedProblems).map(([source, sourceProblems]) => (
              <div key={source} className="py-1">
                {/* Source header */}
                <div className="px-3 py-1 text-xs text-gray-500 font-medium truncate" title={source}>
                  {source}
                </div>
                {/* Problems in this source */}
                {sourceProblems.map((problem) => {
                  const config = PROBLEM_SEVERITY_CONFIG[problem.severity]
                  return (
                    <div
                      key={problem.id}
                      className={`px-3 py-1.5 hover:bg-white/5 cursor-pointer flex items-start gap-2`}
                      title={`${problem.code}: ${problem.message}`}
                    >
                      <span className={`flex-shrink-0 mt-0.5 ${config.color}`}>
                        {config.icon}
                      </span>
                      <div className="flex-1 min-w-0">
                        <div className="text-sm text-gray-200 break-words">
                          {problem.message}
                        </div>
                        <div className="flex items-center gap-2 mt-0.5 text-xs text-gray-500">
                          <span className={`px-1 rounded ${config.bgColor} ${config.color}`}>
                            {problem.code}
                          </span>
                          {problem.field && (
                            <span className="text-gray-400">
                              {problem.field}
                            </span>
                          )}
                          {problem.line !== undefined && (
                            <span>
                              Ln {problem.line}{problem.column !== undefined && `, Col ${problem.column}`}
                            </span>
                          )}
                          {problem.nodeId && (
                            <span className="text-gray-400 truncate" title={problem.nodeId}>
                              {problem.nodeId}
                            </span>
                          )}
                        </div>
                      </div>
                    </div>
                  )
                })}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

function ExecutionsTab() {
  const { executions, executionsLoading, clearExecutions, loadExecutions, selectedNode } = useFunctionsContext()

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-3 py-1 border-b border-white/10">
        <span className="text-xs text-gray-400">
          {executionsLoading ? 'Loading...' : `${executions.length} executions`}
        </span>
        <div className="flex items-center gap-1">
          <button
            onClick={loadExecutions}
            disabled={executionsLoading || !selectedNode}
            className="p-1 text-gray-400 hover:text-white hover:bg-white/10 rounded disabled:opacity-50"
            title="Refresh executions"
          >
            {executionsLoading ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <RefreshCw className="w-3 h-3" />
            )}
          </button>
          <button
            onClick={clearExecutions}
            className="p-1 text-gray-400 hover:text-white hover:bg-white/10 rounded"
            title="Clear executions"
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </div>
      </div>
      <div className="flex-1 overflow-auto">
        {!selectedNode ? (
          <div className="text-gray-500 text-center py-4 text-sm">Select a function to view executions</div>
        ) : executionsLoading && executions.length === 0 ? (
          <div className="text-gray-500 text-center py-4 text-sm flex items-center justify-center gap-2">
            <Loader2 className="w-4 h-4 animate-spin" />
            Loading executions...
          </div>
        ) : executions.length === 0 ? (
          <div className="text-gray-500 text-center py-4 text-sm">No executions yet</div>
        ) : (
          <div className="divide-y divide-white/5">
            {executions.map((exec) => (
              <div key={exec.execution_id} className="px-3 py-2 hover:bg-white/5">
                <div className="flex items-center gap-2">
                  {EXECUTION_STATUS_ICONS[exec.status]}
                  <span className="text-sm text-white">{exec.function_path}</span>
                  {exec.trigger_name && (
                    <span className="text-xs text-gray-500">via {exec.trigger_name}</span>
                  )}
                </div>
                <div className="flex items-center gap-3 mt-1 text-xs text-gray-400">
                  <span>{new Date(exec.started_at).toLocaleString()}</span>
                  {exec.duration_ms && <span>{exec.duration_ms}ms</span>}
                  {exec.error && (
                    <span className="text-red-400 truncate">{exec.error}</span>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

export function OutputPanel() {
  const [activeTab, setActiveTab] = useState<TabId>('output')

  const tabs: { id: TabId; label: string; icon: React.ReactNode }[] = [
    { id: 'output', label: 'Output', icon: <Terminal className="w-4 h-4" /> },
    { id: 'problems', label: 'Problems', icon: <AlertTriangle className="w-4 h-4" /> },
    { id: 'executions', label: 'Executions', icon: <History className="w-4 h-4" /> },
  ]

  return (
    <div className="h-full flex flex-col">
      {/* Tab bar */}
      <div className="flex items-center border-b border-white/10">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`
              flex items-center gap-1.5 px-3 py-1.5 text-sm
              ${activeTab === tab.id
                ? 'text-white border-b-2 border-primary-500'
                : 'text-gray-400 hover:text-white'
              }
            `}
          >
            {tab.icon}
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="flex-1 min-h-0">
        {activeTab === 'output' && <OutputTab />}
        {activeTab === 'problems' && <ProblemsTab />}
        {activeTab === 'executions' && <ExecutionsTab />}
      </div>
    </div>
  )
}
