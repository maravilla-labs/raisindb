/**
 * ErrorDetails Component
 *
 * Displays execution errors with code, message, stack trace, and line/column info.
 */

import { AlertTriangle, Copy, Check } from 'lucide-react'
import { useState } from 'react'

export interface ExecutionError {
  code: string
  message: string
  stack_trace?: string
  line?: number
  column?: number
}

interface ErrorDetailsProps {
  error: ExecutionError | string
  /** Compact mode for inline display */
  compact?: boolean
}

export default function ErrorDetails({ error, compact = false }: ErrorDetailsProps) {
  const [copied, setCopied] = useState(false)

  // Handle string errors (simple error messages)
  const errorObj: ExecutionError = typeof error === 'string'
    ? parseErrorString(error)
    : error

  const copyToClipboard = () => {
    const text = formatErrorForCopy(errorObj)
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }

  if (compact) {
    return (
      <div className="text-sm text-red-300 bg-red-500/10 border border-red-500/20 rounded p-2">
        <div className="flex items-start gap-2">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />
          <div className="flex-1 min-w-0">
            {errorObj.code && (
              <span className="font-mono text-xs bg-red-500/20 px-1 rounded mr-2">
                {errorObj.code}
              </span>
            )}
            <span className="break-words">{errorObj.message}</span>
            {errorObj.line && (
              <span className="text-red-400/70 text-xs ml-2">
                at line {errorObj.line}{errorObj.column ? `:${errorObj.column}` : ''}
              </span>
            )}
          </div>
        </div>
        {errorObj.stack_trace && (
          <pre className="mt-2 text-xs text-red-300/70 font-mono whitespace-pre-wrap break-all overflow-x-auto max-h-32 overflow-y-auto bg-black/20 p-2 rounded">
            {errorObj.stack_trace}
          </pre>
        )}
      </div>
    )
  }

  return (
    <div className="bg-red-500/10 border border-red-500/20 rounded-lg overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 bg-red-500/10 border-b border-red-500/20">
        <div className="flex items-center gap-2">
          <AlertTriangle className="w-5 h-5 text-red-400" />
          <span className="text-red-300 font-semibold">Execution Error</span>
          {errorObj.code && (
            <span className="font-mono text-xs bg-red-500/30 text-red-200 px-2 py-0.5 rounded">
              {errorObj.code}
            </span>
          )}
        </div>
        <button
          onClick={copyToClipboard}
          className="p-1.5 text-red-400 hover:text-red-300 hover:bg-red-500/20 rounded transition-colors"
          title="Copy error details"
        >
          {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
        </button>
      </div>

      {/* Error Message */}
      <div className="p-4">
        <p className="text-red-200 text-sm leading-relaxed">
          {errorObj.message}
        </p>

        {/* Location Info */}
        {(errorObj.line || errorObj.column) && (
          <div className="mt-2 text-xs text-red-400/70">
            Location: {errorObj.line && `line ${errorObj.line}`}
            {errorObj.column && `, column ${errorObj.column}`}
          </div>
        )}

        {/* Stack Trace */}
        {errorObj.stack_trace && (
          <div className="mt-4">
            <div className="text-xs text-red-400/70 mb-1 font-medium">Stack Trace:</div>
            <pre className="text-xs text-red-300/80 font-mono whitespace-pre-wrap break-all overflow-x-auto max-h-48 overflow-y-auto bg-black/30 p-3 rounded border border-red-500/10">
              {formatStackTrace(errorObj.stack_trace)}
            </pre>
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * Parse an error string into an ExecutionError object
 * Handles formats like:
 * - "Internal error: [JS] cannot read property 'node_path' of undefined\n    at handleUserMessage (eval_script:27:42)"
 * - "Function execution failed: TIMEOUT"
 */
function parseErrorString(errorStr: string): ExecutionError {
  // Try to extract error code
  let code = 'RUNTIME_ERROR'
  let message = errorStr
  let stackTrace: string | undefined
  let line: number | undefined
  let column: number | undefined

  // Check for known error codes
  const codeMatch = errorStr.match(/\[([A-Z_]+)\]/)
  if (codeMatch) {
    code = codeMatch[1]
  }

  // Check for [JS] prefix indicating JavaScript error
  if (errorStr.includes('[JS]')) {
    code = 'JS_ERROR'
    message = errorStr.replace(/.*\[JS\]\s*/, '')
  }

  // Check for "Internal error:" prefix
  if (errorStr.includes('Internal error:')) {
    message = errorStr.replace(/.*Internal error:\s*/, '')
  }

  // Check for TIMEOUT
  if (errorStr.includes('TIMEOUT') || errorStr.includes('timed out')) {
    code = 'TIMEOUT'
  }

  // Extract stack trace (lines starting with "at ")
  const lines = message.split('\n')
  const messageLines: string[] = []
  const stackLines: string[] = []

  for (const l of lines) {
    if (l.trim().startsWith('at ')) {
      stackLines.push(l)
      // Try to extract line:column from stack trace
      const locMatch = l.match(/:(\d+):(\d+)/)
      if (locMatch && !line) {
        line = parseInt(locMatch[1], 10)
        column = parseInt(locMatch[2], 10)
      }
    } else {
      messageLines.push(l)
    }
  }

  if (stackLines.length > 0) {
    message = messageLines.join('\n').trim()
    stackTrace = stackLines.join('\n')
  }

  return { code, message, stack_trace: stackTrace, line, column }
}

/**
 * Format stack trace for display
 */
function formatStackTrace(trace: string): string {
  // Clean up and format the stack trace
  return trace
    .split('\n')
    .map(line => line.trim())
    .filter(line => line.length > 0)
    .join('\n')
}

/**
 * Format error for clipboard copy
 */
function formatErrorForCopy(error: ExecutionError): string {
  let text = `Error: ${error.code}\n${error.message}`
  if (error.line) {
    text += `\nLocation: line ${error.line}`
    if (error.column) text += `, column ${error.column}`
  }
  if (error.stack_trace) {
    text += `\n\nStack Trace:\n${error.stack_trace}`
  }
  return text
}
