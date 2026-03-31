/**
 * SQL query support with template literals
 */

import { SqlQueryPayload, SqlResult } from './protocol';

/**
 * SQL query builder with template literal support
 */
export class SqlQuery {
  private sendRequest: (payload: SqlQueryPayload) => Promise<unknown>;

  constructor(sendRequest: (payload: SqlQueryPayload) => Promise<unknown>) {
    this.sendRequest = sendRequest;
  }

  /**
   * Execute a SQL query using template literals
   *
   * @param strings - Template literal strings
   * @param values - Template literal values (automatically parameterized)
   * @returns SQL query result
   *
   * @example
   * ```typescript
   * const results = await sql`SELECT * FROM nodes WHERE node_type = ${nodeType}`;
   * ```
   */
  async query(strings: TemplateStringsArray, ...values: unknown[]): Promise<SqlResult> {
    // Build parameterized query
    let query = strings[0];
    for (let i = 0; i < values.length; i++) {
      query += `$${i + 1}` + strings[i + 1];
    }

    const payload: SqlQueryPayload = {
      query,
      params: values,
    };

    const result = await this.sendRequest(payload);
    return result as SqlResult;
  }

  /**
   * Execute a raw SQL query with explicit parameters
   *
   * @param query - SQL query string with $1, $2, etc. placeholders
   * @param params - Query parameters
   * @returns SQL query result
   *
   * @example
   * ```typescript
   * const results = await sql.execute(
   *   'SELECT * FROM nodes WHERE node_type = $1 AND created_at > $2',
   *   ['page', '2024-01-01']
   * );
   * ```
   */
  async execute(query: string, params?: unknown[]): Promise<SqlResult> {
    const payload: SqlQueryPayload = {
      query,
      params,
    };

    const result = await this.sendRequest(payload);
    return result as SqlResult;
  }

  /**
   * Execute a raw SQL query without parameters
   * Use with caution - prefer parameterized queries for safety
   *
   * @param query - SQL query string
   * @returns SQL query result
   */
  async raw(query: string): Promise<SqlResult> {
    return this.execute(query);
  }
}

/**
 * Helper function to create SQL template literal handler
 *
 * @param sendRequest - Function to send SQL query request
 * @returns Template literal function
 */
export function createSqlHandler(
  sendRequest: (payload: SqlQueryPayload) => Promise<unknown>
): (strings: TemplateStringsArray, ...values: unknown[]) => Promise<SqlResult> {
  const sqlQuery = new SqlQuery(sendRequest);
  return (strings: TemplateStringsArray, ...values: unknown[]) =>
    sqlQuery.query(strings, ...values);
}
