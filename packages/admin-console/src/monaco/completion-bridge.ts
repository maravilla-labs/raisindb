/**
 * Completion Bridge
 *
 * Provides async communication with the WASM worker for completions
 * and function signatures.
 */

import type { FunctionSignature } from './schema-cache'

// =============================================================================
// Types
// =============================================================================

export interface CompletionItem {
  label: string
  kind: CompletionKind
  detail?: string
  documentation?: string
  insert_text: string
  insert_text_format: InsertTextFormat
  sort_text?: string
  filter_text?: string
}

export type CompletionKind =
  | 'keyword'
  | 'table'
  | 'column'
  | 'function'
  | 'aggregate'
  | 'snippet'
  | 'type'
  | 'alias'
  | 'operator'

export type InsertTextFormat = 'plaintext' | 'snippet'

export interface CompletionResult {
  items: CompletionItem[]
  is_incomplete: boolean
}

interface WasmFunctionSignature {
  name: string
  params: string[]
  return_type: string
  category: string
  is_deterministic: boolean
}

// =============================================================================
// Message Types
// =============================================================================

interface CompletionRequestBody {
  type: 'completion'
  sql: string
  offset: number
}

interface SignatureRequestBody {
  type: 'signatures'
  functionName: string
}

interface AllFunctionsRequestBody {
  type: 'all_functions'
}

type WorkerRequestBody = CompletionRequestBody | SignatureRequestBody | AllFunctionsRequestBody

interface CompletionResponse {
  type: 'completion'
  id: number
  result: CompletionResult | null
}

interface SignatureResponse {
  type: 'signatures'
  id: number
  result: WasmFunctionSignature[] | null
}

interface AllFunctionsResponse {
  type: 'all_functions'
  id: number
  result: WasmFunctionSignature[] | null
}

type WorkerResponse = CompletionResponse | SignatureResponse | AllFunctionsResponse

// =============================================================================
// Completion Bridge
// =============================================================================

export class CompletionBridge {
  private worker: Worker
  private requestId = 0
  private pending = new Map<
    number,
    {
      resolve: (value: unknown) => void
      reject: (error: Error) => void
    }
  >()

  constructor(worker: Worker) {
    this.worker = worker
    this.worker.addEventListener('message', this.handleMessage.bind(this))
  }

  private handleMessage(event: MessageEvent<WorkerResponse>) {
    const data = event.data
    if (!data || typeof data.id !== 'number') return

    const pending = this.pending.get(data.id)
    if (!pending) return

    this.pending.delete(data.id)

    switch (data.type) {
      case 'completion':
        pending.resolve(data.result)
        break
      case 'signatures':
        pending.resolve(data.result)
        break
      case 'all_functions':
        pending.resolve(data.result)
        break
    }
  }

  private sendRequest<T>(request: WorkerRequestBody): Promise<T> {
    const id = ++this.requestId
    const fullRequest = { ...request, id }

    return new Promise((resolve, reject) => {
      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      })

      // Set timeout to reject stale requests
      setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id)
          reject(new Error('Request timeout'))
        }
      }, 5000)

      this.worker.postMessage(fullRequest)
    })
  }

  /**
   * Get completions at cursor position
   */
  async getCompletions(sql: string, offset: number): Promise<CompletionResult | null> {
    return this.sendRequest<CompletionResult | null>({
      type: 'completion',
      sql,
      offset,
    })
  }

  /**
   * Get function signatures by name
   */
  async getSignatures(functionName: string): Promise<FunctionSignature[] | null> {
    const result = await this.sendRequest<WasmFunctionSignature[] | null>({
      type: 'signatures',
      functionName,
    })

    if (!result) return null

    return result.map((sig) => ({
      name: sig.name,
      params: sig.params,
      returnType: sig.return_type,
      category: sig.category,
      isDeterministic: sig.is_deterministic,
    }))
  }

  /**
   * Get all function signatures for cache initialization
   */
  async getAllFunctions(): Promise<FunctionSignature[]> {
    const result = await this.sendRequest<WasmFunctionSignature[] | null>({
      type: 'all_functions',
    })

    if (!result) return []

    return result.map((sig) => ({
      name: sig.name,
      params: sig.params,
      returnType: sig.return_type,
      category: sig.category,
      isDeterministic: sig.is_deterministic,
    }))
  }

  /**
   * Clean up pending requests
   */
  dispose() {
    for (const [, pending] of this.pending) {
      pending.reject(new Error('Bridge disposed'))
    }
    this.pending.clear()
  }
}

// =============================================================================
// Singleton
// =============================================================================

let completionBridge: CompletionBridge | null = null

/**
 * Initialize the completion bridge with a worker
 */
export function initializeCompletionBridge(worker: Worker): CompletionBridge {
  if (completionBridge) {
    completionBridge.dispose()
  }
  completionBridge = new CompletionBridge(worker)
  return completionBridge
}

/**
 * Get the completion bridge (must be initialized first)
 */
export function getCompletionBridge(): CompletionBridge | null {
  return completionBridge
}
