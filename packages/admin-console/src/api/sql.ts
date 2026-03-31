import { api } from './client'

export interface SqlQueryRequest {
  sql: string
  params?: unknown[]
}

export interface SqlQueryResponse {
  columns: string[]
  rows: Record<string, any>[]
  row_count: number
  execution_time_ms: number
  explain_plan?: string
}

/**
 * Check if the SQL response represents an async job submission
 *
 * When bulk DML operations (UPDATE/DELETE) with complex WHERE clauses are executed,
 * the server returns a single row with job_id, status, and message columns instead
 * of actual data results.
 */
export function isJobResponse(response: SqlQueryResponse): boolean {
  return (
    response.row_count === 1 &&
    response.columns.includes('job_id') &&
    response.columns.includes('status') &&
    response.columns.includes('message')
  )
}

/**
 * Extract job ID from an async job response
 */
export function extractJobId(response: SqlQueryResponse): string | null {
  if (!isJobResponse(response)) return null
  return response.rows[0]?.job_id ?? null
}

export const sqlApi = {
  /**
   * Execute a SQL query against the repository
   *
   * @param repo - Repository identifier
   * @param sql - SQL query to execute (workspace comes from FROM clause)
   * @param params - Optional parameters for parameterized queries ($1, $2, etc.)
   * @returns Query results with columns and rows
   */
  executeQuery: async (repo: string, sql: string, params?: unknown[]): Promise<SqlQueryResponse> => {
    const body: SqlQueryRequest = { sql }
    if (params && params.length > 0) {
      body.params = params
    }
    return api.post<SqlQueryResponse>(`/api/sql/${repo}`, body)
  }
}
