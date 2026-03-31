/**
 * SQL Validation using WASM
 *
 * Provides real-time SQL validation in Monaco editor by running the
 * RaisinDB SQL parser compiled to WebAssembly in a Web Worker.
 * Also provides semantic completion and signature help functionality.
 */

export { useWasmValidator } from './useWasmValidator'
export type { FunctionSignature } from './useWasmValidator'
export type {
  ValidationError,
  ValidationResult,
  ValidatorWorkerRequest,
  ValidatorWorkerResponse,
  ColumnDef,
  TableDef,
  CompletionResult,
  CompletionItem,
  FunctionSignatureInfo,
} from './types'
