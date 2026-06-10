/**
 * Inbox API
 *
 * API functions for the human-in-the-loop task inbox. Tasks are created by
 * flow wait steps (approval / input / review / action) and completing a task
 * resumes the waiting flow instance.
 */

import { getAuthHeaders } from './client'

// Types

/** Task type discriminator */
export type InboxTaskType = 'approval' | 'input' | 'review' | 'action'

/** Task lifecycle status */
export type InboxTaskStatus = 'pending' | 'completed' | 'expired' | 'cancelled'

/** Option for approval tasks (rendered as a button) */
export interface InboxTaskOption {
  value: string
  label: string
  style?: 'default' | 'success' | 'danger' | 'warning'
}

/** A human-in-the-loop task (node properties flattened + id + path) */
export interface InboxTask {
  id: string
  path: string
  task_type: InboxTaskType
  title: string
  description?: string
  assignee: string
  status: InboxTaskStatus
  /** 1-5, 5 highest */
  priority?: number
  /** Approval task options */
  options?: InboxTaskOption[]
  /** JSON schema for input tasks */
  input_schema?: Record<string, unknown>
  flow_instance_id?: string
  step_id?: string
  created_at?: string
  due_at?: string
  response?: Record<string, unknown>
  completed_by?: string
  responded_at?: string
  /** Set when the task was escalated from a (failed) agent decision */
  escalated_from?: string
  escalation_reason?: string
}

export interface ListInboxTasksResponse {
  assignee: string
  count: number
  tasks: InboxTask[]
}

export interface ListInboxTasksOptions {
  /** Filter by status */
  status?: InboxTaskStatus
  /** View another assignee's inbox (admins only), e.g. `/users/x` */
  assignee?: string
}

export interface CompleteInboxTaskResponse {
  task_id: string
  task_path: string
  status: string
  /** Present when completing the task resumed a waiting flow */
  flow?: {
    instance_id: string
    job_id: string
  }
}

// API Functions

/**
 * List inbox tasks for the current user (or another assignee, admins only).
 *
 * Tasks are sorted pending-first, then priority desc, then due_at asc.
 */
export async function listInboxTasks(
  repo: string,
  opts?: ListInboxTasksOptions
): Promise<ListInboxTasksResponse> {
  const params = new URLSearchParams()
  if (opts?.status) params.set('status', opts.status)
  if (opts?.assignee) params.set('assignee', opts.assignee)
  const query = params.toString()

  const response = await fetch(`/api/inbox/${repo}${query ? `?${query}` : ''}`, {
    headers: { ...getAuthHeaders() },
  })

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }

  return response.json()
}

/**
 * Get a single inbox task by ID
 */
export async function getInboxTask(
  repo: string,
  taskId: string
): Promise<InboxTask> {
  const response = await fetch(`/api/inbox/${repo}/tasks/${taskId}`, {
    headers: { ...getAuthHeaders() },
  })

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }

  return response.json()
}

/**
 * Complete an inbox task.
 *
 * - Approval tasks expect `{ action: "<option value>", comment?: string }`
 * - Input tasks expect the form values object
 * - Review/action tasks expect an acknowledgement, e.g. `{ acknowledged: true }`
 *
 * Completing a task resumes the waiting flow instance (returned as `flow`).
 */
export async function completeInboxTask(
  repo: string,
  taskId: string,
  taskResponse: Record<string, unknown>
): Promise<CompleteInboxTaskResponse> {
  const response = await fetch(`/api/inbox/${repo}/tasks/${taskId}/complete`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...getAuthHeaders() },
    body: JSON.stringify({ response: taskResponse }),
  })

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }

  return response.json()
}

// Convenience exports
export const inboxApi = {
  listInboxTasks,
  getInboxTask,
  completeInboxTask,
}
