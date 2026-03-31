/**
 * useExecutionStorage Hook
 *
 * Manages local execution history using IndexedDB.
 * Stores execution results for later review in the Executions tab.
 */

import { useState, useEffect, useCallback } from 'react'
import type { LogEntry } from '../types'

// IndexedDB database name and version
const DB_NAME = 'raisindb-functions'
const DB_VERSION = 1
const STORE_NAME = 'executions'
const MAX_EXECUTIONS = 100

/** Local execution record */
export interface LocalExecution {
  id: string
  execution_id: string
  file_path: string
  file_id: string
  file_name: string
  handler: string
  input: unknown
  input_node_id?: string
  started_at: string
  ended_at: string
  duration_ms: number
  success: boolean
  result?: unknown
  error?: string
  logs: LogEntry[]
}

// Open IndexedDB database
function openDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION)

    request.onerror = () => reject(request.error)
    request.onsuccess = () => resolve(request.result)

    request.onupgradeneeded = (event) => {
      const db = (event.target as IDBOpenDBRequest).result

      // Create executions store if it doesn't exist
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        const store = db.createObjectStore(STORE_NAME, { keyPath: 'id' })
        store.createIndex('file_path', 'file_path', { unique: false })
        store.createIndex('started_at', 'started_at', { unique: false })
      }
    }
  })
}

// Get all executions
async function getAllExecutions(): Promise<LocalExecution[]> {
  const db = await openDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readonly')
    const store = tx.objectStore(STORE_NAME)
    const index = store.index('started_at')
    const request = index.openCursor(null, 'prev') // Descending order

    const executions: LocalExecution[] = []

    request.onerror = () => reject(request.error)
    request.onsuccess = () => {
      const cursor = request.result
      if (cursor && executions.length < MAX_EXECUTIONS) {
        executions.push(cursor.value)
        cursor.continue()
      } else {
        db.close()
        resolve(executions)
      }
    }
  })
}

// Get executions by file path
async function getExecutionsByPath(filePath: string): Promise<LocalExecution[]> {
  const db = await openDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readonly')
    const store = tx.objectStore(STORE_NAME)
    const index = store.index('file_path')
    const request = index.getAll(filePath)

    request.onerror = () => reject(request.error)
    request.onsuccess = () => {
      db.close()
      // Sort by started_at descending
      const sorted = (request.result as LocalExecution[]).sort(
        (a, b) => new Date(b.started_at).getTime() - new Date(a.started_at).getTime()
      )
      resolve(sorted)
    }
  })
}

// Get execution by ID
async function getExecution(id: string): Promise<LocalExecution | null> {
  const db = await openDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readonly')
    const store = tx.objectStore(STORE_NAME)
    const request = store.get(id)

    request.onerror = () => reject(request.error)
    request.onsuccess = () => {
      db.close()
      resolve(request.result || null)
    }
  })
}

// Save execution
async function saveExecution(execution: LocalExecution): Promise<void> {
  const db = await openDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readwrite')
    const store = tx.objectStore(STORE_NAME)

    // Add the new execution
    const addRequest = store.put(execution)

    addRequest.onerror = () => reject(addRequest.error)

    tx.oncomplete = async () => {
      db.close()
      // Clean up old executions
      await cleanupOldExecutions()
      resolve()
    }
  })
}

// Delete execution
async function deleteExecution(id: string): Promise<void> {
  const db = await openDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readwrite')
    const store = tx.objectStore(STORE_NAME)
    const request = store.delete(id)

    request.onerror = () => reject(request.error)
    tx.oncomplete = () => {
      db.close()
      resolve()
    }
  })
}

// Clear all executions
async function clearAllExecutions(): Promise<void> {
  const db = await openDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readwrite')
    const store = tx.objectStore(STORE_NAME)
    const request = store.clear()

    request.onerror = () => reject(request.error)
    tx.oncomplete = () => {
      db.close()
      resolve()
    }
  })
}

// Clean up old executions to keep only MAX_EXECUTIONS
async function cleanupOldExecutions(): Promise<void> {
  const executions = await getAllExecutions()
  if (executions.length > MAX_EXECUTIONS) {
    const toDelete = executions.slice(MAX_EXECUTIONS)
    for (const exec of toDelete) {
      await deleteExecution(exec.id)
    }
  }
}

/** Hook for managing execution storage */
export function useExecutionStorage() {
  const [executions, setExecutions] = useState<LocalExecution[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<Error | null>(null)

  // Load executions on mount
  useEffect(() => {
    const load = async () => {
      try {
        const data = await getAllExecutions()
        setExecutions(data)
      } catch (err) {
        setError(err instanceof Error ? err : new Error('Failed to load executions'))
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  // Save a new execution
  const save = useCallback(async (execution: LocalExecution) => {
    try {
      await saveExecution(execution)
      setExecutions(prev => [execution, ...prev.slice(0, MAX_EXECUTIONS - 1)])
    } catch (err) {
      console.error('Failed to save execution:', err)
    }
  }, [])

  // Delete an execution
  const remove = useCallback(async (id: string) => {
    try {
      await deleteExecution(id)
      setExecutions(prev => prev.filter(e => e.id !== id))
    } catch (err) {
      console.error('Failed to delete execution:', err)
    }
  }, [])

  // Clear all executions
  const clear = useCallback(async () => {
    try {
      await clearAllExecutions()
      setExecutions([])
    } catch (err) {
      console.error('Failed to clear executions:', err)
    }
  }, [])

  // Get executions for a specific file
  const getByPath = useCallback(async (filePath: string) => {
    try {
      return await getExecutionsByPath(filePath)
    } catch (err) {
      console.error('Failed to get executions by path:', err)
      return []
    }
  }, [])

  // Get a specific execution
  const getById = useCallback(async (id: string) => {
    try {
      return await getExecution(id)
    } catch (err) {
      console.error('Failed to get execution:', err)
      return null
    }
  }, [])

  // Refresh executions from DB
  const refresh = useCallback(async () => {
    try {
      const data = await getAllExecutions()
      setExecutions(data)
    } catch (err) {
      console.error('Failed to refresh executions:', err)
    }
  }, [])

  return {
    executions,
    loading,
    error,
    save,
    remove,
    clear,
    getByPath,
    getById,
    refresh,
  }
}

/** Create a new execution record */
export function createExecution(params: {
  executionId: string
  filePath: string
  fileId: string
  fileName: string
  handler: string
  input: unknown
  inputNodeId?: string
}): LocalExecution {
  const now = new Date().toISOString()
  return {
    id: params.executionId,
    execution_id: params.executionId,
    file_path: params.filePath,
    file_id: params.fileId,
    file_name: params.fileName,
    handler: params.handler,
    input: params.input,
    input_node_id: params.inputNodeId,
    started_at: now,
    ended_at: now,
    duration_ms: 0,
    success: false,
    result: undefined,
    error: undefined,
    logs: [],
  }
}
