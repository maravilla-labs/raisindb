/**
 * React hook for SQL validation using WASM in a Web Worker
 *
 * Provides non-blocking SQL validation with debouncing and Monaco marker integration.
 * Also provides completion and signature help functionality via the same worker.
 */

import { useRef, useEffect, useCallback, useState } from 'react'
import type { editor } from 'monaco-editor'
import type {
  ValidatorWorkerResponse,
  ValidationResult,
  ValidationError,
  TableDef,
  CompletionResult,
  FunctionSignatureInfo,
} from './types'

interface UseWasmValidatorOptions {
  /** Debounce delay in milliseconds (default: 300) */
  debounceMs?: number
}

/** Function signature for schema cache (camelCase) */
export interface FunctionSignature {
  name: string
  params: string[]
  returnType: string
  category: string
  isDeterministic: boolean
}

interface UseWasmValidatorReturn {
  /** Function to validate SQL (debounced) */
  validate: (sql: string) => void
  /** Whether the WASM module is ready */
  isReady: boolean
  /** Last validation result */
  lastResult: ValidationResult | null
  /** Set the table catalog for semantic validation, with optional callback on success */
  setTableCatalog: (catalog: TableDef[], onSet?: () => void) => void
  /** Clear the table catalog */
  clearTableCatalog: () => void
  /** Get completions at cursor position (for semantic completion) */
  getCompletions: (sql: string, offset: number) => Promise<CompletionResult | null>
  /** Get function signatures by name (for signature help) */
  getSignatures: (functionName: string) => Promise<FunctionSignature[] | null>
  /** Get all function signatures (for cache initialization) */
  getAllFunctions: () => Promise<FunctionSignature[]>
}

/**
 * Hook for validating SQL using WASM in a Web Worker
 *
 * @param editorRef - Reference to the Monaco editor instance
 * @param monaco - Monaco editor module
 * @param options - Configuration options
 */
export function useWasmValidator(
  editorRef: React.RefObject<editor.IStandaloneCodeEditor | null>,
  monaco: typeof import('monaco-editor') | null,
  options: UseWasmValidatorOptions = {}
): UseWasmValidatorReturn {
  const { debounceMs = 300 } = options

  const workerRef = useRef<Worker | null>(null)
  const [isReady, setIsReady] = useState(false)
  const [lastResult, setLastResult] = useState<ValidationResult | null>(null)
  const debounceRef = useRef<number | null>(null)
  const requestIdRef = useRef(0)
  const latestRequestIdRef = useRef(0)
  const catalogSetCallbackRef = useRef<(() => void) | null>(null)

  // Pending completion/signature requests
  const pendingRequests = useRef<
    Map<
      number,
      {
        resolve: (value: unknown) => void
        reject: (error: Error) => void
        type: 'completion' | 'signatures' | 'all_functions'
      }
    >
  >(new Map())

  /**
   * Handle validation result from worker
   */
  const handleResult = useCallback(
    (data: ValidatorWorkerResponse) => {
      if (data.type === 'ready') {
        setIsReady(true)
        return
      }

      if (data.type === 'catalog_set') {
        if (data.success && catalogSetCallbackRef.current) {
          catalogSetCallbackRef.current()
          catalogSetCallbackRef.current = null
        }
        return
      }

      // Handle completion responses
      if (data.type === 'completion' || data.type === 'signatures' || data.type === 'all_functions') {
        const pending = pendingRequests.current.get(data.id)
        if (pending) {
          pendingRequests.current.delete(data.id)
          if ('error' in data && data.error) {
            pending.reject(new Error(data.error))
          } else {
            pending.resolve(data.result ?? null)
          }
        }
        return
      }

      if (data.type === 'result') {
        // Ignore stale results
        if (data.id !== undefined && data.id < latestRequestIdRef.current) {
          return
        }

        if (data.error) {
          console.error('SQL validation error:', data.error)
          return
        }

        if (data.result) {
          setLastResult(data.result)

          // Update Monaco markers
          if (editorRef.current && monaco) {
            const model = editorRef.current.getModel()
            if (model) {
              const markers = data.result.errors.map((err: ValidationError) => ({
                severity:
                  err.severity === 'error'
                    ? monaco.MarkerSeverity.Error
                    : monaco.MarkerSeverity.Warning,
                message: err.message,
                startLineNumber: err.line,
                startColumn: err.column,
                endLineNumber: err.end_line,
                endColumn: err.end_column,
              }))

              monaco.editor.setModelMarkers(model, 'sql-validator', markers)
            }
          }
        }
      }
    },
    [monaco, editorRef]
  )

  /**
   * Initialize the Web Worker
   */
  useEffect(() => {
    // Create worker using Vite's worker import syntax
    workerRef.current = new Worker(
      new URL('./validator.worker.ts', import.meta.url),
      { type: 'module' }
    )

    workerRef.current.onmessage = (event: MessageEvent<ValidatorWorkerResponse>) => {
      handleResult(event.data)
    }

    workerRef.current.onerror = (error) => {
      console.error('Validation worker error:', error)
    }

    return () => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current)
      }
      workerRef.current?.terminate()
    }
  }, [handleResult])

  /**
   * Validate SQL with debouncing
   */
  const validate = useCallback(
    (sql: string) => {
      if (!isReady || !workerRef.current) {
        return
      }

      // Clear previous debounce timer
      if (debounceRef.current) {
        clearTimeout(debounceRef.current)
      }

      // Debounce validation
      debounceRef.current = window.setTimeout(() => {
        const id = ++requestIdRef.current
        latestRequestIdRef.current = id

        workerRef.current?.postMessage({
          type: 'validate',
          id,
          sql,
        })
      }, debounceMs)
    },
    [isReady, debounceMs]
  )

  /**
   * Set the table catalog for semantic validation
   * @param catalog - Array of table definitions
   * @param onSet - Optional callback called when catalog is confirmed set
   */
  const setTableCatalog = useCallback(
    (catalog: TableDef[], onSet?: () => void) => {
      if (!isReady || !workerRef.current) {
        return
      }

      // Store callback to be called when catalog_set response is received
      if (onSet) {
        catalogSetCallbackRef.current = onSet
      }

      workerRef.current.postMessage({
        type: 'set_catalog',
        catalog,
      })
    },
    [isReady]
  )

  /**
   * Clear the table catalog
   */
  const clearTableCatalog = useCallback(() => {
    if (!workerRef.current) {
      return
    }

    workerRef.current.postMessage({
      type: 'clear_catalog',
    })
  }, [])

  /**
   * Get completions at cursor position
   */
  const getCompletions = useCallback(
    (sql: string, offset: number): Promise<CompletionResult | null> => {
      if (!isReady || !workerRef.current) {
        return Promise.resolve(null)
      }

      const id = ++requestIdRef.current

      return new Promise((resolve, reject) => {
        pendingRequests.current.set(id, {
          resolve: resolve as (value: unknown) => void,
          reject,
          type: 'completion',
        })

        // Set timeout to reject stale requests
        setTimeout(() => {
          if (pendingRequests.current.has(id)) {
            pendingRequests.current.delete(id)
            reject(new Error('Completion request timeout'))
          }
        }, 5000)

        workerRef.current?.postMessage({
          type: 'completion',
          id,
          sql,
          offset,
        })
      })
    },
    [isReady]
  )

  /**
   * Get function signatures by name
   */
  const getSignatures = useCallback(
    (functionName: string): Promise<FunctionSignature[] | null> => {
      if (!isReady || !workerRef.current) {
        return Promise.resolve(null)
      }

      const id = ++requestIdRef.current

      return new Promise((resolve, reject) => {
        pendingRequests.current.set(id, {
          resolve: (value: unknown) => {
            // Transform from snake_case to camelCase
            const result = value as FunctionSignatureInfo[] | null
            if (!result) {
              resolve(null)
              return
            }
            resolve(
              result.map((sig) => ({
                name: sig.name,
                params: sig.params,
                returnType: sig.return_type,
                category: sig.category,
                isDeterministic: sig.is_deterministic,
              }))
            )
          },
          reject,
          type: 'signatures',
        })

        // Set timeout to reject stale requests
        setTimeout(() => {
          if (pendingRequests.current.has(id)) {
            pendingRequests.current.delete(id)
            reject(new Error('Signature request timeout'))
          }
        }, 5000)

        workerRef.current?.postMessage({
          type: 'signatures',
          id,
          functionName,
        })
      })
    },
    [isReady]
  )

  /**
   * Get all function signatures for cache initialization
   */
  const getAllFunctions = useCallback((): Promise<FunctionSignature[]> => {
    if (!isReady || !workerRef.current) {
      return Promise.resolve([])
    }

    const id = ++requestIdRef.current

    return new Promise((resolve, reject) => {
      pendingRequests.current.set(id, {
        resolve: (value: unknown) => {
          // Transform from snake_case to camelCase
          const result = value as FunctionSignatureInfo[] | null
          if (!result) {
            resolve([])
            return
          }
          resolve(
            result.map((sig) => ({
              name: sig.name,
              params: sig.params,
              returnType: sig.return_type,
              category: sig.category,
              isDeterministic: sig.is_deterministic,
            }))
          )
        },
        reject,
        type: 'all_functions',
      })

      // Set timeout to reject stale requests
      setTimeout(() => {
        if (pendingRequests.current.has(id)) {
          pendingRequests.current.delete(id)
          reject(new Error('All functions request timeout'))
        }
      }, 5000)

      workerRef.current?.postMessage({
        type: 'all_functions',
        id,
      })
    })
  }, [isReady])

  return {
    validate,
    isReady,
    lastResult,
    setTableCatalog,
    clearTableCatalog,
    getCompletions,
    getSignatures,
    getAllFunctions,
  }
}
