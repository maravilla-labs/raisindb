import { useState, useEffect, useCallback } from 'react'
import { useParams } from 'react-router-dom'
import { sqlApi, type SqlQueryResponse, isJobResponse, extractJobId } from '../api/sql'
import { managementApi } from '../api/management'
import { ApiError } from '../api/client'
import { Play, Database, Clock, Hash, AlertCircle, Loader2, History, X, ChevronRight, ChevronDown, HelpCircle, RefreshCw, CheckCircle, XCircle, Timer } from 'lucide-react'
import { SqlEditor } from '../monaco/SqlEditor'
import { SqlHelpSidebar } from '../components/SqlHelpSidebar'
import { QueryPlanView } from '../components/QueryPlanView'

interface QueryHistoryItem {
  query: string
  timestamp: number
  repo: string
}

const HISTORY_STORAGE_KEY = 'sql_query_history'
const MAX_HISTORY_ITEMS = 50

function getQueryHistory(): QueryHistoryItem[] {
  try {
    const stored = localStorage.getItem(HISTORY_STORAGE_KEY)
    return stored ? JSON.parse(stored) : []
  } catch {
    return []
  }
}

function saveQueryToHistory(query: string, repo: string) {
  try {
    const history = getQueryHistory()
    const newItem: QueryHistoryItem = {
      query: query.trim(),
      timestamp: Date.now(),
      repo,
    }

    // Remove duplicates and add to front
    const filtered = history.filter(item => item.query !== newItem.query || item.repo !== newItem.repo)
    const updated = [newItem, ...filtered].slice(0, MAX_HISTORY_ITEMS)

    localStorage.setItem(HISTORY_STORAGE_KEY, JSON.stringify(updated))
  } catch (err) {
    console.error('Failed to save query history:', err)
  }
}

// Job polling interval in milliseconds
const JOB_POLL_INTERVAL = 2000

export function SqlQuery() {
  const { repo } = useParams<{ repo: string }>()
  const [sql, setSql] = useState('SELECT * FROM nodes LIMIT 10')
  const [result, setResult] = useState<SqlQueryResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [showHistory, setShowHistory] = useState(false)
  const [showHelp, setShowHelp] = useState(false)
  const [queryHistory, setQueryHistory] = useState<QueryHistoryItem[]>([])

  // Job tracking state
  const [activeJobId, setActiveJobId] = useState<string | null>(null)
  const [jobStatus, setJobStatus] = useState<string | null>(null)
  const [jobStartTime, setJobStartTime] = useState<number | null>(null)
  const [jobPolling, setJobPolling] = useState(false)

  useEffect(() => {
    setQueryHistory(getQueryHistory())
  }, [])

  // Poll job status when there's an active job
  const pollJobStatus = useCallback(async (jobId: string) => {
    try {
      const response = await managementApi.getJobStatus(jobId)
      if (response.data) {
        // data is the status string directly (e.g., "Running", "Completed", { Failed: "error" })
        const status = response.data
        const statusStr = typeof status === 'string' ? status : 'Failed'
        setJobStatus(statusStr)

        // Check if job is completed or failed
        if (statusStr === 'Completed' || statusStr === 'Cancelled' || statusStr === 'Failed') {
          setJobPolling(false)
        }
      }
    } catch (err) {
      console.error('Failed to poll job status:', err)
    }
  }, [])

  // Set up polling interval
  useEffect(() => {
    if (!activeJobId || !jobPolling) return

    // Initial poll
    pollJobStatus(activeJobId)

    // Set up interval
    const interval = setInterval(() => {
      pollJobStatus(activeJobId)
    }, JOB_POLL_INTERVAL)

    return () => clearInterval(interval)
  }, [activeJobId, jobPolling, pollJobStatus])

  const handleExecuteQuery = async () => {
    if (!repo || !sql.trim()) {
      setError('Please enter a SQL query')
      return
    }

    try {
      setLoading(true)
      setError(null)
      // Clear any previous job state
      setActiveJobId(null)
      setJobStatus(null)
      setJobStartTime(null)
      setJobPolling(false)

      const response = await sqlApi.executeQuery(repo, sql)

      // Check if this is an async job response
      if (isJobResponse(response)) {
        const jobId = extractJobId(response)
        if (jobId) {
          setActiveJobId(jobId)
          setJobStatus('Scheduled')
          setJobStartTime(Date.now())
          setJobPolling(true)
          setResult(null) // Don't show table for job response
        }
      } else {
        setResult(response)
      }

      // Save to history on successful execution
      saveQueryToHistory(sql, repo)
      setQueryHistory(getQueryHistory())
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message)
      } else {
        setError('Failed to execute query')
      }
      setResult(null)
      setActiveJobId(null)
      setJobStatus(null)
      setJobStartTime(null)
    } finally {
      setLoading(false)
    }
  }

  const handleClearJob = () => {
    setActiveJobId(null)
    setJobStatus(null)
    setJobStartTime(null)
    setJobPolling(false)
  }

  const loadQueryFromHistory = (query: string) => {
    setSql(query)
    setShowHistory(false)
  }

  const handleInsertExample = (exampleSql: string) => {
    setSql(exampleSql)
    // Don't auto-close help - user might want to reference more examples
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      handleExecuteQuery()
    }
  }

  return (
    <div className="h-full flex flex-col overflow-hidden animate-fade-in">
      {/* Header */}
      <div className="flex items-center justify-between flex-shrink-0 mb-6">
        <div className="flex items-center gap-3">
          <div className="p-2 bg-primary-500/10 rounded-lg border border-primary-500/20">
            <Database className="w-5 h-5 text-primary-400" />
          </div>
          <div>
            <h1 className="text-2xl font-bold text-white">SQL Query Console</h1>
            <p className="text-sm text-zinc-400 mt-0.5">
              Execute SQL queries against {repo}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2 text-xs px-3 py-2 bg-yellow-500/10 border border-yellow-500/20 rounded-lg">
          <AlertCircle className="w-4 h-4 text-yellow-400" />
          <span className="text-yellow-300 font-medium">Development Only</span>
        </div>
      </div>

      {/* SQL Editor Section - Glass card with fixed height */}
      <div className="flex flex-col flex-shrink-0 bg-white/5 backdrop-blur-md border border-white/10 rounded-xl shadow-lg overflow-hidden mb-4" style={{ height: '300px' }}>
        <div className="flex items-center justify-between px-6 py-3 border-b border-white/10 bg-gradient-to-r from-black/30 to-black/20">
          <div className="flex items-center gap-2 text-sm text-zinc-400">
            <span className="font-mono">{repo}</span>
            <span>•</span>
            <span className="text-zinc-500">Ctrl+Enter to execute</span>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowHelp(!showHelp)}
              className="flex items-center gap-2 px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10
                       text-zinc-300 rounded-lg transition-all font-medium
                       focus:outline-none focus:ring-2 focus:ring-primary-500/50
                       active:scale-95"
            >
              <HelpCircle className="w-4 h-4" />
              <span>Help</span>
            </button>

            <button
              onClick={() => setShowHistory(!showHistory)}
              className="flex items-center gap-2 px-3 py-2 bg-white/5 hover:bg-white/10 border border-white/10
                       text-zinc-300 rounded-lg transition-all font-medium
                       focus:outline-none focus:ring-2 focus:ring-primary-500/50
                       active:scale-95 relative"
            >
              <History className="w-4 h-4" />
              <span>History</span>
              {queryHistory.length > 0 && (
                <span className="absolute -top-1 -right-1 bg-primary-500 text-white text-xs rounded-full w-5 h-5 flex items-center justify-center shadow-lg">
                  {queryHistory.length}
                </span>
              )}
            </button>

            <button
              onClick={handleExecuteQuery}
              disabled={loading || !sql.trim()}
              className="flex items-center gap-2 px-5 py-2 bg-gradient-to-r from-primary-500 to-primary-600 hover:from-primary-600 hover:to-primary-700
                       disabled:from-zinc-700 disabled:to-zinc-700 disabled:text-zinc-500 disabled:cursor-not-allowed
                       text-white rounded-lg transition-all font-medium shadow-lg shadow-primary-500/20
                       focus:outline-none focus:ring-2 focus:ring-primary-500/50
                       active:scale-95"
            >
              {loading ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  <span>Executing...</span>
                </>
              ) : (
                <>
                  <Play className="w-4 h-4" />
                  <span>Execute Query</span>
                </>
              )}
            </button>
          </div>
        </div>

        {/* Monaco Editor - Takes remaining space in fixed section */}
        <div className="flex-1 min-h-0" onKeyDown={handleKeyDown}>
          <SqlEditor
            value={sql}
            onChange={setSql}
            height="100%"
            repo={repo}
          />
        </div>
      </div>

      {/* Query History Panel */}
      {showHistory && (
        <div className="flex-shrink-0 bg-white/5 backdrop-blur-md border border-white/10 rounded-xl shadow-lg overflow-hidden mb-4" style={{ maxHeight: '250px' }}>
          <div className="flex items-center justify-between px-6 py-3 border-b border-white/10 bg-gradient-to-r from-black/30 to-black/20">
            <div className="flex items-center gap-2">
              <History className="w-4 h-4 text-primary-400" />
              <h3 className="text-sm font-semibold text-white">Query History</h3>
            </div>
            <button
              onClick={() => setShowHistory(false)}
              className="text-zinc-400 hover:text-white transition-colors p-1.5 hover:bg-white/10 rounded-lg"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
          <div className="overflow-y-auto bg-black/10" style={{ maxHeight: '200px' }}>
            {queryHistory.length === 0 ? (
              <div className="p-8 text-center text-zinc-500">
                <History className="w-12 h-12 mx-auto mb-3 opacity-30" />
                <p className="text-sm font-medium">No query history yet</p>
                <p className="text-xs text-zinc-600 mt-1">Your executed queries will appear here</p>
              </div>
            ) : (
              <div className="p-3">
                {queryHistory.map((item, idx) => (
                  <button
                    key={idx}
                    onClick={() => loadQueryFromHistory(item.query)}
                    className="w-full text-left px-4 py-3 hover:bg-white/10 rounded-lg transition-all
                             border border-transparent hover:border-white/20 mb-2 group"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <code className="text-sm text-zinc-300 font-mono flex-1 line-clamp-2 group-hover:text-white transition-colors">
                        {item.query}
                      </code>
                      <div className="flex flex-col items-end gap-1 flex-shrink-0">
                        <span className="text-xs text-zinc-500 font-mono px-2 py-0.5 bg-white/5 rounded">{item.repo}</span>
                        <span className="text-xs text-zinc-600">
                          {new Date(item.timestamp).toLocaleString()}
                        </span>
                      </div>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      )}

      {/* Job Progress Tracker */}
      {activeJobId && (
        <div className="flex-shrink-0 bg-white/5 backdrop-blur-md border border-white/10 rounded-xl shadow-lg overflow-hidden mb-4">
          <div className="px-6 py-3 border-b border-white/10 bg-gradient-to-r from-black/30 to-black/20 flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Timer className="w-4 h-4 text-primary-400" />
              <h2 className="text-sm font-semibold text-white">Background Job</h2>
            </div>
            <button
              onClick={handleClearJob}
              className="text-zinc-400 hover:text-white transition-colors p-1.5 hover:bg-white/10 rounded-lg"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
          <div className="p-6">
            <JobProgressTracker
              jobId={activeJobId}
              status={jobStatus}
              startTime={jobStartTime}
              isPolling={jobPolling}
              onRefresh={() => pollJobStatus(activeJobId)}
            />
          </div>
        </div>
      )}

      {/* Results Panel - Takes remaining space and scrolls */}
      {(result || error) && (
        <div className="flex-1 flex flex-col bg-white/5 backdrop-blur-md border border-white/10 rounded-xl shadow-lg overflow-hidden min-h-0">
          <div className="px-6 py-3 border-b border-white/10 bg-gradient-to-r from-black/30 to-black/20 flex items-center justify-between flex-shrink-0">
            <div className="flex items-center gap-2">
              <Database className="w-4 h-4 text-primary-400" />
              <h2 className="text-sm font-semibold text-white">Results</h2>
            </div>
            {result && (
              <div className="flex items-center gap-4 text-xs text-zinc-400">
                <div className="flex items-center gap-1.5 px-2 py-1 bg-white/5 rounded">
                  <Hash className="w-3.5 h-3.5 text-primary-400" />
                  <span className="font-medium">{result.row_count} rows</span>
                </div>
                <div className="flex items-center gap-1.5 px-2 py-1 bg-white/5 rounded">
                  <Clock className="w-3.5 h-3.5 text-green-400" />
                  <span className="font-medium">{result.execution_time_ms}ms</span>
                </div>
              </div>
            )}
          </div>

          <div className="flex-1 overflow-auto min-h-0 min-w-0">
            {error ? (
              <div className="p-6">
                <div className="bg-gradient-to-br from-red-500/10 to-red-600/5 border border-red-500/30 rounded-xl shadow-lg p-5">
                  <div className="flex items-start gap-3">
                    <div className="p-2 bg-red-500/20 rounded-lg">
                      <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0" />
                    </div>
                    <div className="flex-1">
                      <h3 className="text-sm font-semibold text-red-400 mb-2">Query Error</h3>
                      <pre className="text-sm text-red-300 font-mono whitespace-pre-wrap bg-black/20 rounded-lg p-3 border border-red-500/20">
                        {error}
                      </pre>
                    </div>
                  </div>
                </div>
              </div>
            ) : result && result.explain_plan ? (
              <div className="p-6">
                <QueryPlanView plan={result.explain_plan} />
              </div>
            ) : result && result.row_count === 0 ? (
              <div className="p-12 text-center text-zinc-500">
                <Database className="w-16 h-16 mx-auto mb-4 opacity-30" />
                <p className="text-sm font-medium">No results found</p>
                <p className="text-xs text-zinc-600 mt-1">Your query executed successfully but returned no rows</p>
              </div>
            ) : result ? (
              <div className="bg-black/30 rounded-lg m-4">
                <table className="w-full border-collapse">
                  <thead>
                    <tr className="border-b border-white/10">
                      {result.columns.map((col) => (
                        <th
                          key={col}
                          className="text-left px-4 py-3 text-xs font-semibold text-primary-300 uppercase tracking-wider bg-black/50 whitespace-nowrap sticky top-0 z-10 first:rounded-tl-lg last:rounded-tr-lg"
                        >
                          {col}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody className="bg-black/20">
                    {result.rows.map((row, idx) => (
                      <tr
                        key={idx}
                        className="border-b border-white/5 hover:bg-white/5 transition-colors"
                      >
                        {result.columns.map((col) => (
                          <td
                            key={col}
                            className="px-4 py-3 text-sm align-top"
                          >
                            {renderCellValue(row[col])}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : null}
          </div>
        </div>
      )}

      {/* Help Sidebar */}
      <SqlHelpSidebar
        isOpen={showHelp}
        onClose={() => setShowHelp(false)}
        onInsertExample={handleInsertExample}
        repo={repo || 'unknown'}
      />
    </div>
  )
}

// JSON Tree Component for displaying objects
function JsonTree({ data, level = 0 }: { data: any; level?: number }) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})

  const toggleExpand = (key: string) => {
    setExpanded((prev) => ({ ...prev, [key]: !prev[key] }))
  }

  if (data === null || data === undefined) {
    return <span className="text-zinc-500 italic">null</span>
  }

  if (typeof data !== 'object') {
    if (typeof data === 'string') {
      return <span className="text-green-400">&quot;{data}&quot;</span>
    }
    if (typeof data === 'number') {
      return <span className="text-blue-400">{data}</span>
    }
    if (typeof data === 'boolean') {
      return <span className="text-purple-400">{data.toString()}</span>
    }
    return <span className="text-zinc-300">{String(data)}</span>
  }

  const isArray = Array.isArray(data)
  const entries = isArray ? data.map((v, i) => [String(i), v]) : Object.entries(data)

  if (entries.length === 0) {
    return <span className="text-zinc-500">{isArray ? '[]' : '{}'}</span>
  }

  return (
    <div className="inline-block text-left">
      <div className="font-mono text-xs">
        <span className="text-zinc-500">{isArray ? '[' : '{'}</span>
        <div className="ml-4">
          {entries.map(([key, value], idx) => {
            const fullKey = `${level}-${key}`
            const isObject = value !== null && typeof value === 'object'
            const isExpanded = expanded[fullKey]

            return (
              <div key={fullKey} className="my-0.5">
                <div className="flex items-start gap-1">
                  {isObject && (
                    <button
                      onClick={() => toggleExpand(fullKey)}
                      className="text-zinc-400 hover:text-white transition-colors p-0.5 -ml-5"
                    >
                      {isExpanded ? (
                        <ChevronDown className="w-3 h-3" />
                      ) : (
                        <ChevronRight className="w-3 h-3" />
                      )}
                    </button>
                  )}
                  {!isObject && <span className="w-4" />}
                  {!isArray && (
                    <span className="text-cyan-400">&quot;{key}&quot;: </span>
                  )}
                  {isObject ? (
                    <div>
                      {!isExpanded ? (
                        <span className="text-zinc-500 cursor-pointer hover:text-zinc-400" onClick={() => toggleExpand(fullKey)}>
                          {Array.isArray(value)
                            ? `[${value.length} items]`
                            : `{${Object.keys(value).length} keys}`}
                        </span>
                      ) : (
                        <JsonTree data={value} level={level + 1} />
                      )}
                    </div>
                  ) : (
                    <JsonTree data={value} level={level + 1} />
                  )}
                  {idx < entries.length - 1 && <span className="text-zinc-500">,</span>}
                </div>
              </div>
            )
          })}
        </div>
        <span className="text-zinc-500">{isArray ? ']' : '}'}</span>
      </div>
    </div>
  )
}

function renderCellValue(value: any): React.ReactNode {
  if (value === null || value === undefined) {
    return <span className="text-zinc-500 italic">∅</span>
  }
  if (typeof value === 'object') {
    return <JsonTree data={value} />
  }
  if (typeof value === 'boolean') {
    return <span className="text-purple-400">{value ? 'true' : 'false'}</span>
  }
  if (typeof value === 'number') {
    return <span className="text-blue-400">{value}</span>
  }
  if (typeof value === 'string') {
    // Check if the string contains newlines (like EXPLAIN plans)
    if (value.includes('\n')) {
      return (
        <pre className="text-zinc-300 font-mono text-xs whitespace-pre-wrap max-w-4xl">
          {value}
        </pre>
      )
    }
    return <span className="text-zinc-300">{value}</span>
  }
  return <span className="text-zinc-300">{String(value)}</span>
}

// Job Progress Tracker Component
interface JobProgressTrackerProps {
  jobId: string
  status: string | null
  startTime: number | null
  isPolling: boolean
  onRefresh: () => void
}

function JobProgressTracker({ jobId, status, startTime, isPolling, onRefresh }: JobProgressTrackerProps) {
  const [elapsedTime, setElapsedTime] = useState<string>('')

  // Update elapsed time every second while job is running
  useEffect(() => {
    if (!startTime || status === 'Completed' || status === 'Failed' || status === 'Cancelled') {
      return
    }

    const updateElapsed = () => {
      const elapsed = Math.round((Date.now() - startTime) / 1000)
      if (elapsed < 60) {
        setElapsedTime(`${elapsed}s`)
      } else if (elapsed < 3600) {
        setElapsedTime(`${Math.floor(elapsed / 60)}m ${elapsed % 60}s`)
      } else {
        setElapsedTime(`${Math.floor(elapsed / 3600)}h ${Math.floor((elapsed % 3600) / 60)}m`)
      }
    }

    updateElapsed()
    const interval = setInterval(updateElapsed, 1000)
    return () => clearInterval(interval)
  }, [startTime, status])

  const getStatusIcon = () => {
    if (!status || status === 'Running' || status === 'Scheduled') {
      return <Loader2 className="w-5 h-5 text-yellow-400 animate-spin" />
    }
    if (status === 'Completed') {
      return <CheckCircle className="w-5 h-5 text-green-400" />
    }
    if (status === 'Cancelled') {
      return <XCircle className="w-5 h-5 text-zinc-400" />
    }
    // Failed status
    return <XCircle className="w-5 h-5 text-red-400" />
  }

  const getStatusColor = (): string => {
    if (!status) return 'yellow'
    switch (status) {
      case 'Scheduled': return 'blue'
      case 'Running': return 'yellow'
      case 'Completed': return 'green'
      case 'Cancelled': return 'gray'
      case 'Failed': return 'red'
      default: return 'yellow'
    }
  }

  const statusColorClasses: Record<string, string> = {
    blue: 'bg-blue-500/20 text-blue-400 border-blue-500/30',
    yellow: 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30',
    green: 'bg-green-500/20 text-green-400 border-green-500/30',
    red: 'bg-red-500/20 text-red-400 border-red-500/30',
    gray: 'bg-zinc-500/20 text-zinc-400 border-zinc-500/30',
  }

  const isJobDone = status === 'Completed' || status === 'Failed' || status === 'Cancelled'

  return (
    <div className="space-y-4">
      {/* Job ID and Status */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          {getStatusIcon()}
          <div>
            <div className="flex items-center gap-2">
              <span className="text-white font-semibold">Background Job</span>
              <code className="text-xs text-zinc-400 bg-black/30 px-2 py-0.5 rounded font-mono">
                {jobId}
              </code>
            </div>
            <div className="flex items-center gap-2 mt-1">
              <span className={`text-xs px-2 py-0.5 rounded border ${statusColorClasses[getStatusColor()]}`}>
                {status || 'Starting...'}
              </span>
              {isPolling && (
                <span className="flex items-center gap-1 text-xs text-zinc-500">
                  <RefreshCw className="w-3 h-3 animate-spin" />
                  Polling...
                </span>
              )}
            </div>
          </div>
        </div>
        <button
          onClick={onRefresh}
          className="flex items-center gap-2 px-3 py-1.5 bg-white/5 hover:bg-white/10 border border-white/10
                   text-zinc-300 rounded-lg transition-all text-sm
                   focus:outline-none focus:ring-2 focus:ring-primary-500/50"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          <span>Refresh</span>
        </button>
      </div>

      {/* Job Details */}
      <div className="grid grid-cols-2 gap-4 p-4 bg-black/20 rounded-lg">
        {startTime && (
          <div>
            <div className="text-xs text-zinc-500 mb-1">Started</div>
            <div className="text-sm text-zinc-300">
              {new Date(startTime).toLocaleTimeString()}
            </div>
          </div>
        )}
        {elapsedTime && (
          <div>
            <div className="text-xs text-zinc-500 mb-1">
              {isJobDone ? 'Duration' : 'Elapsed'}
            </div>
            <div className="text-sm text-zinc-300">{elapsedTime}</div>
          </div>
        )}
      </div>

      {/* Success Message */}
      {status === 'Completed' && (
        <div className="p-4 bg-green-500/10 border border-green-500/30 rounded-lg">
          <div className="flex items-start gap-3">
            <CheckCircle className="w-5 h-5 text-green-400 flex-shrink-0 mt-0.5" />
            <div className="flex-1">
              <div className="text-sm font-semibold text-green-400">Job Completed Successfully</div>
              <div className="text-xs text-zinc-400 mt-1">
                The bulk operation has finished. Check the commit history for details.
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Failed Message */}
      {status === 'Failed' && (
        <div className="p-4 bg-red-500/10 border border-red-500/30 rounded-lg">
          <div className="flex items-start gap-3">
            <XCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
            <div>
              <div className="text-sm font-semibold text-red-400">Job Failed</div>
              <div className="text-xs text-zinc-400 mt-1">
                The bulk operation failed. Check the server logs for details.
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Cancelled Message */}
      {status === 'Cancelled' && (
        <div className="p-4 bg-zinc-500/10 border border-zinc-500/30 rounded-lg">
          <div className="flex items-start gap-3">
            <XCircle className="w-5 h-5 text-zinc-400 flex-shrink-0 mt-0.5" />
            <div>
              <div className="text-sm font-semibold text-zinc-400">Job Cancelled</div>
              <div className="text-xs text-zinc-500 mt-1">
                The bulk operation was cancelled.
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
