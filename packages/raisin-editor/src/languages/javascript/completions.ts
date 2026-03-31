/**
 * RaisinDB JavaScript API completions
 *
 * Provides autocomplete for the raisin.* global object available in functions.
 */

import type { Monaco } from '@monaco-editor/react'
import type { languages, editor, Position } from 'monaco-editor'

export interface RaisinApiCompletion {
  label: string
  kind: languages.CompletionItemKind
  insertText: string
  insertTextRules?: languages.CompletionItemInsertTextRule
  documentation?: string
  detail?: string
}

/**
 * Get completions for raisin.* API
 */
export function getRaisinApiCompletions(
  monaco: Monaco
): RaisinApiCompletion[] {
  const { CompletionItemKind, CompletionItemInsertTextRule } = monaco.languages

  return [
    // raisin namespace
    {
      label: 'raisin',
      kind: CompletionItemKind.Module,
      insertText: 'raisin',
      documentation: 'RaisinDB function API namespace',
      detail: 'RaisinDB API',
    },

    // raisin.nodes - Node operations
    {
      label: 'nodes',
      kind: CompletionItemKind.Property,
      insertText: 'nodes',
      documentation: 'Node CRUD operations',
      detail: 'raisin.nodes',
    },
    {
      label: 'get',
      kind: CompletionItemKind.Method,
      insertText: 'get(${1:workspace}, ${2:path})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Get a node by path\n\nreturns: Promise<Node>',
      detail: 'raisin.nodes.get(workspace, path)',
    },
    {
      label: 'getById',
      kind: CompletionItemKind.Method,
      insertText: 'getById(${1:workspace}, ${2:id})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Get a node by ID\n\nreturns: Promise<Node>',
      detail: 'raisin.nodes.getById(workspace, id)',
    },
    {
      label: 'create',
      kind: CompletionItemKind.Method,
      insertText: 'create(${1:workspace}, ${2:parentPath}, ${3:data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Create a new node\n\ndata: { name, node_type, properties }\nreturns: Promise<Node>',
      detail: 'raisin.nodes.create(workspace, parentPath, data)',
    },
    {
      label: 'update',
      kind: CompletionItemKind.Method,
      insertText: 'update(${1:workspace}, ${2:path}, ${3:data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Update an existing node\n\ndata: { properties }\nreturns: Promise<Node>',
      detail: 'raisin.nodes.update(workspace, path, data)',
    },
    {
      label: 'delete',
      kind: CompletionItemKind.Method,
      insertText: 'delete(${1:workspace}, ${2:path})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Delete a node\n\nreturns: Promise<void>',
      detail: 'raisin.nodes.delete(workspace, path)',
    },
    {
      label: 'query',
      kind: CompletionItemKind.Method,
      insertText: 'query(${1:workspace}, ${2:query})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Query nodes with DSL\n\nreturns: Promise<Node[]>',
      detail: 'raisin.nodes.query(workspace, query)',
    },
    {
      label: 'getChildren',
      kind: CompletionItemKind.Method,
      insertText: 'getChildren(${1:workspace}, ${2:path}, ${3:limit})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Get child nodes\n\nreturns: Promise<Node[]>',
      detail: 'raisin.nodes.getChildren(workspace, path, limit?)',
    },

    // raisin.sql - SQL operations
    {
      label: 'sql',
      kind: CompletionItemKind.Property,
      insertText: 'sql',
      documentation: 'SQL query operations',
      detail: 'raisin.sql',
    },
    {
      label: 'query',
      kind: CompletionItemKind.Method,
      insertText: 'query(${1:sql}, ${2:params})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Run SQL query and return results\n\nreturns: Promise<{ rows, row_count }>',
      detail: 'raisin.sql.query(sql, params)',
    },
    {
      label: 'execute',
      kind: CompletionItemKind.Method,
      insertText: 'execute(${1:sql}, ${2:params})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Run SQL statement and return affected rows\n\nreturns: Promise<number>',
      detail: 'raisin.sql.execute(sql, params)',
    },

    // raisin.http - HTTP operations
    {
      label: 'http',
      kind: CompletionItemKind.Property,
      insertText: 'http',
      documentation: 'HTTP request operations (allowlisted URLs only)',
      detail: 'raisin.http',
    },
    {
      label: 'fetch',
      kind: CompletionItemKind.Method,
      insertText: 'fetch(${1:url}, ${2:{ method: "POST", body: {} \\}})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Make HTTP request (only to allowlisted URLs)\n\noptions: { method, headers, body }\nreturns: Promise<Response>',
      detail: 'raisin.http.fetch(url, options)',
    },

    // raisin.events - Event operations
    {
      label: 'events',
      kind: CompletionItemKind.Property,
      insertText: 'events',
      documentation: 'Event emission',
      detail: 'raisin.events',
    },
    {
      label: 'emit',
      kind: CompletionItemKind.Method,
      insertText: 'emit(${1:eventType}, ${2:data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Emit a custom event\n\nreturns: Promise<void>',
      detail: 'raisin.events.emit(eventType, data)',
    },

    // raisin.context - Execution context (read-only)
    {
      label: 'context',
      kind: CompletionItemKind.Property,
      insertText: 'context',
      documentation: 'Execution context (read-only)',
      detail: 'raisin.context',
    },
    {
      label: 'tenant_id',
      kind: CompletionItemKind.Property,
      insertText: 'tenant_id',
      documentation: 'Current tenant ID',
      detail: 'string',
    },
    {
      label: 'repo_id',
      kind: CompletionItemKind.Property,
      insertText: 'repo_id',
      documentation: 'Current repository ID',
      detail: 'string',
    },
    {
      label: 'branch',
      kind: CompletionItemKind.Property,
      insertText: 'branch',
      documentation: 'Current branch name',
      detail: 'string',
    },
    {
      label: 'workspace_id',
      kind: CompletionItemKind.Property,
      insertText: 'workspace_id',
      documentation: 'Current workspace ID',
      detail: 'string',
    },
    {
      label: 'actor',
      kind: CompletionItemKind.Property,
      insertText: 'actor',
      documentation: 'User/actor ID executing the function',
      detail: 'string',
    },
    {
      label: 'execution_id',
      kind: CompletionItemKind.Property,
      insertText: 'execution_id',
      documentation: 'Unique ID for this execution',
      detail: 'string',
    },

    // Console logging
    {
      label: 'console.log',
      kind: CompletionItemKind.Function,
      insertText: 'console.log(${1:message})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Log info message',
      detail: 'console.log(message)',
    },
    {
      label: 'console.debug',
      kind: CompletionItemKind.Function,
      insertText: 'console.debug(${1:message})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Log debug message',
      detail: 'console.debug(message)',
    },
    {
      label: 'console.warn',
      kind: CompletionItemKind.Function,
      insertText: 'console.warn(${1:message})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Log warning message',
      detail: 'console.warn(message)',
    },
    {
      label: 'console.error',
      kind: CompletionItemKind.Function,
      insertText: 'console.error(${1:message})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Log error message',
      detail: 'console.error(message)',
    },
  ]
}

/**
 * Register RaisinDB JavaScript completion provider
 */
export function registerRaisinJsCompletionProvider(monaco: Monaco): void {
  const completions = getRaisinApiCompletions(monaco)

  monaco.languages.registerCompletionItemProvider('javascript', {
    triggerCharacters: ['.'],
    provideCompletionItems: (model: editor.ITextModel, position: Position) => {
      const textUntilPosition = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      })

      const word = model.getWordUntilPosition(position)
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      }

      // Check what context we're in
      const suggestions: languages.CompletionItem[] = []

      // raisin.* completions
      if (/\braisin\.$/.test(textUntilPosition)) {
        // After "raisin." - show nodes, sql, http, events, context
        const namespaces = ['nodes', 'sql', 'http', 'events', 'context']
        for (const ns of namespaces) {
          const completion = completions.find((c) => c.label === ns && c.kind === monaco.languages.CompletionItemKind.Property)
          if (completion) {
            suggestions.push({
              ...completion,
              range,
            })
          }
        }
      } else if (/\braisin\.nodes\.$/.test(textUntilPosition)) {
        // After "raisin.nodes." - show node methods
        const methods = ['get', 'getById', 'create', 'update', 'delete', 'query', 'getChildren']
        for (const method of methods) {
          const completion = completions.find((c) => c.label === method && c.kind === monaco.languages.CompletionItemKind.Method)
          if (completion) {
            suggestions.push({
              ...completion,
              range,
            })
          }
        }
      } else if (/\braisin\.sql\.$/.test(textUntilPosition)) {
        // After "raisin.sql." - show sql methods
        const methods = ['query', 'execute']
        for (const method of methods) {
          const completion = completions.find((c) => c.label === method && c.kind === monaco.languages.CompletionItemKind.Method)
          if (completion) {
            suggestions.push({
              ...completion,
              range,
            })
          }
        }
      } else if (/\braisin\.http\.$/.test(textUntilPosition)) {
        // After "raisin.http." - show http methods
        const completion = completions.find((c) => c.label === 'fetch')
        if (completion) {
          suggestions.push({
            ...completion,
            range,
          })
        }
      } else if (/\braisin\.events\.$/.test(textUntilPosition)) {
        // After "raisin.events." - show events methods
        const completion = completions.find((c) => c.label === 'emit')
        if (completion) {
          suggestions.push({
            ...completion,
            range,
          })
        }
      } else if (/\braisin\.context\.$/.test(textUntilPosition)) {
        // After "raisin.context." - show context properties
        const props = ['tenant_id', 'repo_id', 'branch', 'workspace_id', 'actor', 'execution_id']
        for (const prop of props) {
          const completion = completions.find((c) => c.label === prop && c.kind === monaco.languages.CompletionItemKind.Property)
          if (completion) {
            suggestions.push({
              ...completion,
              range,
            })
          }
        }
      } else if (/\bconsole\.$/.test(textUntilPosition)) {
        // After "console." - show console methods
        const methods = ['log', 'debug', 'warn', 'error']
        for (const method of methods) {
          const completion = completions.find((c) => c.label === `console.${method}`)
          if (completion) {
            suggestions.push({
              label: method,
              kind: completion.kind,
              insertText: completion.insertText.replace(`console.${method}`, method),
              insertTextRules: completion.insertTextRules,
              documentation: completion.documentation,
              detail: completion.detail,
              range,
            })
          }
        }
      } else if (word.word === '' || /^[a-z]/i.test(word.word)) {
        // Global scope - show raisin and console
        const raisinCompletion = completions.find((c) => c.label === 'raisin')
        if (raisinCompletion) {
          suggestions.push({
            ...raisinCompletion,
            range,
          })
        }
        // Also suggest console.* methods
        for (const c of completions.filter((c) => c.label.startsWith('console.'))) {
          suggestions.push({
            ...c,
            range,
          })
        }
      }

      return { suggestions }
    },
  })
}
