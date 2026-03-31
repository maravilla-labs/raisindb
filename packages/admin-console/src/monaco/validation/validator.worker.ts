/**
 * Web Worker for SQL validation and completion using WASM module
 *
 * This runs in a background thread to avoid blocking the main UI
 * while parsing and validating SQL statements and generating completions.
 */

import init, {
  validate_sql,
  set_table_catalog,
  clear_table_catalog,
  get_completions,
  get_function_signatures,
  get_all_functions,
} from '@raisindb/sql-wasm'
import type {
  ValidatorWorkerRequest,
  ValidatorWorkerResponse,
  ValidationResult,
  CompletionResult,
  FunctionSignatureInfo,
  TableDef,
} from './types'

let wasmReady = false
let pendingCatalog: TableDef[] | null = null
let pendingAllFunctionsId: number | null = null

/**
 * Initialize the WASM module
 */
async function initWasm() {
  try {
    await init()
    wasmReady = true

    // Process any pending catalog that arrived before WASM was ready
    if (pendingCatalog) {
      try {
        set_table_catalog(pendingCatalog)
        console.log('[Worker] Pending table catalog set successfully')
        const response: ValidatorWorkerResponse = { type: 'catalog_set', success: true }
        self.postMessage(response)
      } catch (error) {
        console.error('[Worker] Failed to set pending table catalog:', error)
        const response: ValidatorWorkerResponse = {
          type: 'catalog_set',
          success: false,
          error: String(error),
        }
        self.postMessage(response)
      }
      pendingCatalog = null
    }

    // Process any pending all_functions request
    if (pendingAllFunctionsId !== null) {
      try {
        const result = get_all_functions() as FunctionSignatureInfo[] | null
        console.log('[Worker] Pending all_functions request processed')
        const response: ValidatorWorkerResponse = {
          type: 'all_functions',
          id: pendingAllFunctionsId,
          result: result ?? undefined,
        }
        self.postMessage(response)
      } catch (error) {
        const response: ValidatorWorkerResponse = {
          type: 'all_functions',
          id: pendingAllFunctionsId,
          error: `Function list error: ${error}`,
        }
        self.postMessage(response)
      }
      pendingAllFunctionsId = null
    }

    const response: ValidatorWorkerResponse = { type: 'ready' }
    self.postMessage(response)
  } catch (error) {
    console.error('Failed to initialize WASM module:', error)
    // Can't report error via normal response since we don't have an id
    // Just log it for now
  }
}

/**
 * Handle messages from the main thread
 */
self.onmessage = async (event: MessageEvent<ValidatorWorkerRequest>) => {
  const data = event.data

  if (data.type === 'validate') {
    if (!wasmReady) {
      const response: ValidatorWorkerResponse = {
        type: 'result',
        id: data.id,
        error: 'WASM module not ready',
      }
      self.postMessage(response)
      return
    }

    try {
      const result = validate_sql(data.sql) as ValidationResult
      const response: ValidatorWorkerResponse = {
        type: 'result',
        id: data.id,
        result,
      }
      self.postMessage(response)
    } catch (error) {
      const response: ValidatorWorkerResponse = {
        type: 'result',
        id: data.id,
        error: `Validation error: ${error}`,
      }
      self.postMessage(response)
    }
    return
  }

  if (data.type === 'set_catalog') {
    console.log('[Worker] Received set_catalog:', data.catalog)
    if (!wasmReady) {
      // Queue the catalog to be set once WASM is ready
      console.log('[Worker] WASM not ready, queuing catalog')
      pendingCatalog = data.catalog
      // Don't send response yet - will be sent after WASM init
      return
    }

    try {
      set_table_catalog(data.catalog)
      console.log('[Worker] Table catalog set successfully')
      const response: ValidatorWorkerResponse = {
        type: 'catalog_set',
        success: true,
      }
      self.postMessage(response)
    } catch (error) {
      console.error('[Worker] Failed to set table catalog:', error)
      const response: ValidatorWorkerResponse = {
        type: 'catalog_set',
        success: false,
        error: String(error),
      }
      self.postMessage(response)
    }
    return
  }

  if (data.type === 'clear_catalog') {
    // Clear any pending catalog too
    pendingCatalog = null
    if (wasmReady) {
      clear_table_catalog()
    }
    const response: ValidatorWorkerResponse = { type: 'catalog_cleared' }
    self.postMessage(response)
    return
  }

  // =========================================================================
  // Completion Handlers
  // =========================================================================

  if (data.type === 'completion') {
    if (!wasmReady) {
      const response: ValidatorWorkerResponse = {
        type: 'completion',
        id: data.id,
        error: 'WASM module not ready',
      }
      self.postMessage(response)
      return
    }

    try {
      const result = get_completions(data.sql, data.offset) as CompletionResult | null
      const response: ValidatorWorkerResponse = {
        type: 'completion',
        id: data.id,
        result: result ?? undefined,
      }
      self.postMessage(response)
    } catch (error) {
      const response: ValidatorWorkerResponse = {
        type: 'completion',
        id: data.id,
        error: `Completion error: ${error}`,
      }
      self.postMessage(response)
    }
    return
  }

  if (data.type === 'signatures') {
    if (!wasmReady) {
      const response: ValidatorWorkerResponse = {
        type: 'signatures',
        id: data.id,
        error: 'WASM module not ready',
      }
      self.postMessage(response)
      return
    }

    try {
      const result = get_function_signatures(data.functionName) as FunctionSignatureInfo[] | null
      const response: ValidatorWorkerResponse = {
        type: 'signatures',
        id: data.id,
        result: result ?? undefined,
      }
      self.postMessage(response)
    } catch (error) {
      const response: ValidatorWorkerResponse = {
        type: 'signatures',
        id: data.id,
        error: `Signature error: ${error}`,
      }
      self.postMessage(response)
    }
    return
  }

  if (data.type === 'all_functions') {
    if (!wasmReady) {
      // Queue the request to be processed once WASM is ready
      console.log('[Worker] WASM not ready, queuing all_functions request')
      pendingAllFunctionsId = data.id
      return
    }

    try {
      const result = get_all_functions() as FunctionSignatureInfo[] | null
      const response: ValidatorWorkerResponse = {
        type: 'all_functions',
        id: data.id,
        result: result ?? undefined,
      }
      self.postMessage(response)
    } catch (error) {
      const response: ValidatorWorkerResponse = {
        type: 'all_functions',
        id: data.id,
        error: `Function list error: ${error}`,
      }
      self.postMessage(response)
    }
    return
  }
}

// Initialize WASM on worker start
initWasm()
