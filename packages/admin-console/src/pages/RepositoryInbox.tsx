/**
 * Repository Inbox Page
 *
 * Human-in-the-loop task inbox for workflows. Features:
 * - List of tasks assigned to the current user (pending first)
 * - Status filter tabs (Pending / Completed / All) with pending count
 * - Expandable rows with the full TaskDetail view + completion form
 * - Real-time refresh via the jobs SSE endpoint (flows resume as jobs)
 */

import { useEffect, useState, useMemo, useCallback } from 'react'
import { useParams } from 'react-router-dom'
import { Inbox, ChevronDown, ChevronRight, Clock } from 'lucide-react'
import { listInboxTasks, completeInboxTask, type InboxTask } from '../api/inbox'
import { sseManager } from '../api/management'
import { useToast, ToastContainer } from '../components/Toast'
import TaskTypeBadge from '../components/inbox/TaskTypeBadge'
import PriorityBadge from '../components/inbox/PriorityBadge'
import AssigneeBadge from '../components/inbox/AssigneeBadge'
import TaskDetail, { isTaskOverdue } from '../components/inbox/TaskDetail'

type FilterStatus = 'pending' | 'completed' | 'all'

export default function RepositoryInbox() {
  const { repo } = useParams<{ repo: string }>()
  const [tasks, setTasks] = useState<InboxTask[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)
  const [expandedTaskId, setExpandedTaskId] = useState<string | null>(null)
  const [completingTaskId, setCompletingTaskId] = useState<string | null>(null)
  const [statusFilter, setStatusFilter] = useState<FilterStatus>('pending')
  // 'mine' = the logged-in principal's inbox; 'all' = every assignee
  // (admins only - the server rejects 'all' for non-admins and we fall back)
  const [scope, setScope] = useState<'mine' | 'all'>('mine')
  const [scopeForbidden, setScopeForbidden] = useState(false)
  const { toasts, success: showSuccess, error: showError, closeToast } = useToast()

  // Fetch all tasks (server sorts pending-first, priority desc, due_at asc)
  const fetchTasks = useCallback(async () => {
    if (!repo) return

    try {
      const result = await listInboxTasks(
        repo,
        scope === 'all' ? { assignee: '*' } : undefined
      )
      setTasks(result.tasks)
      setError(null)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to fetch inbox tasks'
      if (scope === 'all' && /403|forbidden|admins/i.test(message)) {
        // Not an admin - hide the All option and fall back to own inbox
        setScopeForbidden(true)
        setScope('mine')
        return
      }
      setError(message)
    } finally {
      setLoading(false)
    }
  }, [repo, scope])

  useEffect(() => {
    fetchTasks()
  }, [fetchTasks])

  // SSE connection for real-time updates via jobs endpoint.
  // Flow jobs finishing means tasks may have been created or resolved.
  useEffect(() => {
    if (!repo) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event) => {
        if (event.job_type === 'FlowInstanceExecution') {
          if (event.status === 'completed' || event.status === 'failed') {
            fetchTasks()
          }
        }
      },
      onOpen: () => setConnected(true),
      onError: () => setConnected(false),
    })

    return cleanup
  }, [repo, fetchTasks])

  const pendingCount = useMemo(
    () => tasks.filter((t) => t.status === 'pending').length,
    [tasks]
  )

  const filteredTasks = useMemo(() => {
    if (statusFilter === 'all') return tasks
    if (statusFilter === 'pending') return tasks.filter((t) => t.status === 'pending')
    return tasks.filter((t) => t.status !== 'pending')
  }, [tasks, statusFilter])

  const handleComplete = async (task: InboxTask, response: Record<string, unknown>) => {
    if (!repo) return

    setCompletingTaskId(task.id)
    try {
      const result = await completeInboxTask(repo, task.id, response)

      // Optimistic update
      setTasks((prev) =>
        prev.map((t) =>
          t.id === task.id
            ? { ...t, status: 'completed' as const, response, responded_at: new Date().toISOString() }
            : t
        )
      )

      showSuccess(
        'Task completed',
        result.flow ? 'Task completed — flow resumed' : undefined
      )

      // Refresh from server (picks up completed_by, flow-driven changes)
      fetchTasks()
    } catch (err) {
      showError(
        'Failed to complete task',
        err instanceof Error ? err.message : 'Unknown error'
      )
    } finally {
      setCompletingTaskId(null)
    }
  }

  if (loading) {
    return (
      <div className="p-8">
        <div className="animate-pulse space-y-4">
          <div className="h-8 bg-white/10 rounded w-64"></div>
          <div className="h-12 bg-white/5 rounded"></div>
          <div className="space-y-2">
            {[1, 2, 3, 4, 5].map((i) => (
              <div key={i} className="h-14 bg-white/5 rounded"></div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-8">
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4 text-red-300">
          {error}
        </div>
      </div>
    )
  }

  return (
    <div className="p-6 md:p-8 max-w-[1600px] mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold text-white flex items-center gap-3">
            <Inbox className="w-7 h-7 text-purple-400" />
            Inbox
          </h1>
          <p className="text-zinc-400 text-sm mt-1">Tasks waiting for your decision</p>
        </div>
        <div className="flex items-center gap-3">
          {/* Connection Status */}
          <div className="flex items-center gap-2 px-3 py-1.5 bg-white/5 rounded-lg">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'} animate-pulse`}></div>
            <span className="text-xs text-zinc-400">{connected ? 'Live' : 'Offline'}</span>
          </div>
        </div>
      </div>

      {/* Status Filter Tabs + Scope Toggle */}
      <div className="flex gap-1 mb-4 items-center">
        {(['pending', 'completed', 'all'] as FilterStatus[]).map((status) => (
          <button
            key={status}
            onClick={() => setStatusFilter(status)}
            className={`px-3 py-1.5 text-sm rounded-lg transition-colors capitalize flex items-center gap-2 ${
              statusFilter === status
                ? 'bg-purple-500 text-white'
                : 'bg-white/5 text-zinc-400 hover:text-white'
            }`}
          >
            {status}
            {status === 'pending' && pendingCount > 0 && (
              <span
                className={`px-1.5 py-0.5 rounded-full text-xs font-medium ${
                  statusFilter === 'pending'
                    ? 'bg-white/20 text-white'
                    : 'bg-purple-500/20 text-purple-400'
                }`}
              >
                {pendingCount}
              </span>
            )}
          </button>
        ))}

        {/* Scope: my inbox vs all assignees (admins) */}
        {!scopeForbidden && (
          <div className="ml-auto flex items-center gap-1 bg-white/5 rounded-lg p-0.5">
            {(['mine', 'all'] as const).map((s) => (
              <button
                key={s}
                onClick={() => setScope(s)}
                title={
                  s === 'all'
                    ? 'All assignees (admins only)'
                    : 'Tasks assigned to you'
                }
                className={`px-2.5 py-1 text-xs rounded-md transition-colors ${
                  scope === s
                    ? 'bg-white/10 text-white'
                    : 'text-zinc-500 hover:text-white'
                }`}
              >
                {s === 'mine' ? 'My tasks' : 'All assignees'}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Results Count */}
      <div className="text-xs text-zinc-500 mb-3">
        {filteredTasks.length} task{filteredTasks.length !== 1 ? 's' : ''}
      </div>

      {/* Task List */}
      {filteredTasks.length === 0 ? (
        <div className="bg-white/5 border border-white/10 rounded-xl p-12 text-center">
          <Inbox className="w-12 h-12 text-zinc-600 mx-auto mb-4" />
          <p className="text-zinc-400">
            {statusFilter === 'pending' ? 'No pending tasks — all caught up' : 'No tasks'}
          </p>
        </div>
      ) : (
        <div className="bg-white/5 border border-white/10 rounded-xl overflow-hidden">
          {/* Table Header */}
          <div className="grid grid-cols-[auto_1fr_110px_110px_160px_150px_100px] gap-4 px-4 py-3 bg-white/5 border-b border-white/10 text-xs text-zinc-500 font-medium uppercase tracking-wider">
            <div className="w-6"></div>
            <div>Task</div>
            <div>Type</div>
            <div>Priority</div>
            <div>Assignee</div>
            <div>Due</div>
            <div>Created</div>
          </div>

          {/* Table Body */}
          <div className="divide-y divide-white/5">
            {filteredTasks.map((task) => {
              const isExpanded = expandedTaskId === task.id
              const overdue = isTaskOverdue(task)
              const isDone = task.status !== 'pending'

              return (
                <div key={task.id}>
                  {/* Row */}
                  <div
                    className={`grid grid-cols-[auto_1fr_110px_110px_160px_150px_100px] gap-4 px-4 py-3 items-center hover:bg-white/5 cursor-pointer transition-colors ${
                      overdue ? 'bg-red-500/5' : ''
                    } ${isDone ? 'opacity-60' : ''}`}
                    onClick={() => setExpandedTaskId(isExpanded ? null : task.id)}
                  >
                    {/* Expand Icon */}
                    <div className="text-zinc-500">
                      {isExpanded ? (
                        <ChevronDown className="w-5 h-5" />
                      ) : (
                        <ChevronRight className="w-5 h-5" />
                      )}
                    </div>

                    {/* Task Info */}
                    <div className="min-w-0">
                      <div className="text-sm text-white font-medium truncate" title={task.title}>
                        {task.title}
                      </div>
                      <div className="text-xs text-zinc-500 truncate capitalize">
                        {task.status}
                        {task.escalated_from && (
                          <span className="ml-2 text-amber-400 normal-case">Escalated</span>
                        )}
                      </div>
                    </div>

                    {/* Type */}
                    <div>
                      <TaskTypeBadge taskType={task.task_type} />
                    </div>

                    {/* Priority */}
                    <div>
                      <PriorityBadge priority={task.priority} />
                    </div>

                    {/* Assignee */}
                    <div className="min-w-0">
                      <AssigneeBadge assignee={task.assignee} />
                    </div>

                    {/* Due */}
                    <div className={`text-xs flex items-center gap-1.5 ${overdue ? 'text-red-400 font-medium' : 'text-zinc-500'}`}>
                      {task.due_at ? (
                        <>
                          {overdue && <Clock className="w-3.5 h-3.5" />}
                          {new Date(task.due_at).toLocaleString()}
                        </>
                      ) : (
                        '-'
                      )}
                    </div>

                    {/* Created */}
                    <div className="text-xs text-zinc-500">
                      {task.created_at ? new Date(task.created_at).toLocaleTimeString() : '-'}
                    </div>
                  </div>

                  {/* Expanded Content */}
                  {isExpanded && (
                    <div className="px-4 pb-4 bg-black/20 border-t border-white/5">
                      <div className="pt-4 pl-9 max-w-3xl">
                        <TaskDetail
                          task={task}
                          onComplete={(response) => handleComplete(task, response)}
                          busy={completingTaskId === task.id}
                        />
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
