/**
 * RaisinDB Starlark/Python API completions
 *
 * Provides autocomplete for the raisin.* global object available in functions.
 * Uses snake_case naming convention for Python/Starlark style.
 */

import type { Monaco } from '@monaco-editor/react'
import type { languages, editor, Position } from 'monaco-editor'
import { STARLARK_LANGUAGE_ID } from './config'

export interface RaisinApiCompletion {
  label: string
  kind: languages.CompletionItemKind
  insertText: string
  insertTextRules?: languages.CompletionItemInsertTextRule
  documentation?: string
  detail?: string
}

/**
 * Get completions for raisin.* API (snake_case style)
 */
export function getRaisinStarlarkCompletions(
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

    // ===== raisin.nodes - Node operations =====
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
      documentation: 'Get a node by path\n\nArgs:\n  workspace: Workspace name\n  path: Node path\n\nReturns: dict | None',
      detail: 'raisin.nodes.get(workspace, path)',
    },
    {
      label: 'get_by_id',
      kind: CompletionItemKind.Method,
      insertText: 'get_by_id(${1:workspace}, ${2:id})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Get a node by ID\n\nArgs:\n  workspace: Workspace name\n  id: Node ID\n\nReturns: dict | None',
      detail: 'raisin.nodes.get_by_id(workspace, id)',
    },
    {
      label: 'create',
      kind: CompletionItemKind.Method,
      insertText: 'create(${1:workspace}, ${2:parent_path}, ${3:data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Create a new node\n\nArgs:\n  workspace: Workspace name\n  parent_path: Parent path\n  data: { name, node_type, properties }\n\nReturns: dict',
      detail: 'raisin.nodes.create(workspace, parent_path, data)',
    },
    {
      label: 'update',
      kind: CompletionItemKind.Method,
      insertText: 'update(${1:workspace}, ${2:path}, ${3:data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Update an existing node\n\nArgs:\n  workspace: Workspace name\n  path: Node path\n  data: { properties }\n\nReturns: dict',
      detail: 'raisin.nodes.update(workspace, path, data)',
    },
    {
      label: 'delete',
      kind: CompletionItemKind.Method,
      insertText: 'delete(${1:workspace}, ${2:path})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Delete a node\n\nArgs:\n  workspace: Workspace name\n  path: Node path\n\nReturns: bool',
      detail: 'raisin.nodes.delete(workspace, path)',
    },
    {
      label: 'query',
      kind: CompletionItemKind.Method,
      insertText: 'query(${1:workspace}, ${2:query})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Query nodes with DSL\n\nArgs:\n  workspace: Workspace name\n  query: Query dict { node_type, limit, ... }\n\nReturns: list[dict]',
      detail: 'raisin.nodes.query(workspace, query)',
    },
    {
      label: 'get_children',
      kind: CompletionItemKind.Method,
      insertText: 'get_children(${1:workspace}, ${2:path}, ${3:limit})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Get child nodes\n\nArgs:\n  workspace: Workspace name\n  path: Parent path\n  limit: Max results (optional)\n\nReturns: list[dict]',
      detail: 'raisin.nodes.get_children(workspace, path, limit=None)',
    },
    {
      label: 'update_property',
      kind: CompletionItemKind.Method,
      insertText: 'update_property(${1:workspace}, ${2:node_path}, ${3:property_path}, ${4:value})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Update a specific property on a node\n\nArgs:\n  workspace: Workspace name\n  node_path: Node path\n  property_path: Dot-notation property path\n  value: New value',
      detail: 'raisin.nodes.update_property(workspace, node_path, property_path, value)',
    },
    {
      label: 'move',
      kind: CompletionItemKind.Method,
      insertText: 'move(${1:workspace}, ${2:node_path}, ${3:new_parent_path})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Move a node to a new parent\n\nArgs:\n  workspace: Workspace name\n  node_path: Current path\n  new_parent_path: New parent path\n\nReturns: dict',
      detail: 'raisin.nodes.move(workspace, node_path, new_parent_path)',
    },

    // ===== raisin.sql - SQL operations =====
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
      documentation: 'Run SQL query and return results\n\nArgs:\n  sql: SQL query with $1, $2, ... placeholders\n  params: List of parameter values\n\nReturns: { rows, row_count, columns }',
      detail: 'raisin.sql.query(sql, params=[])',
    },
    {
      label: 'execute',
      kind: CompletionItemKind.Method,
      insertText: 'execute(${1:sql}, ${2:params})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Run SQL statement and return affected rows\n\nArgs:\n  sql: SQL statement\n  params: List of parameter values\n\nReturns: int',
      detail: 'raisin.sql.execute(sql, params=[])',
    },

    // ===== raisin.http - HTTP operations (requests-style) =====
    {
      label: 'http',
      kind: CompletionItemKind.Property,
      insertText: 'http',
      documentation: 'HTTP request operations (allowlisted URLs only)',
      detail: 'raisin.http',
    },
    {
      label: 'get',
      kind: CompletionItemKind.Method,
      insertText: 'get(${1:url}, headers=${2:{}})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Make HTTP GET request\n\nArgs:\n  url: Request URL (must be allowlisted)\n  headers: Optional headers dict\n\nReturns: Response { status_code, headers, json, text }',
      detail: 'raisin.http.get(url, headers={})',
    },
    {
      label: 'post',
      kind: CompletionItemKind.Method,
      insertText: 'post(${1:url}, json=${2:{}}, headers=${3:{}})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Make HTTP POST request\n\nArgs:\n  url: Request URL (must be allowlisted)\n  json: Request body as dict\n  headers: Optional headers dict\n\nReturns: Response { status_code, headers, json, text }',
      detail: 'raisin.http.post(url, json={}, headers={})',
    },
    {
      label: 'put',
      kind: CompletionItemKind.Method,
      insertText: 'put(${1:url}, json=${2:{}}, headers=${3:{}})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Make HTTP PUT request\n\nArgs:\n  url: Request URL (must be allowlisted)\n  json: Request body as dict\n  headers: Optional headers dict\n\nReturns: Response { status_code, headers, json, text }',
      detail: 'raisin.http.put(url, json={}, headers={})',
    },
    {
      label: 'patch',
      kind: CompletionItemKind.Method,
      insertText: 'patch(${1:url}, json=${2:{}}, headers=${3:{}})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Make HTTP PATCH request\n\nArgs:\n  url: Request URL (must be allowlisted)\n  json: Request body as dict\n  headers: Optional headers dict\n\nReturns: Response { status_code, headers, json, text }',
      detail: 'raisin.http.patch(url, json={}, headers={})',
    },
    {
      label: 'delete',
      kind: CompletionItemKind.Method,
      insertText: 'delete(${1:url}, headers=${2:{}})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Make HTTP DELETE request\n\nArgs:\n  url: Request URL (must be allowlisted)\n  headers: Optional headers dict\n\nReturns: Response { status_code, headers, json, text }',
      detail: 'raisin.http.delete(url, headers={})',
    },

    // ===== raisin.ai - AI operations =====
    {
      label: 'ai',
      kind: CompletionItemKind.Property,
      insertText: 'ai',
      documentation: 'AI completion and embedding operations',
      detail: 'raisin.ai',
    },
    {
      label: 'completion',
      kind: CompletionItemKind.Method,
      insertText: 'completion(${1:request})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Call AI completion\n\nArgs:\n  request: { model, messages, temperature, max_tokens }\n\nReturns: { message, model, usage }',
      detail: 'raisin.ai.completion(request)',
    },
    {
      label: 'embed',
      kind: CompletionItemKind.Method,
      insertText: 'embed(${1:request})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Generate embeddings\n\nArgs:\n  request: { model, input, input_type }\n\nReturns: { embedding, model, dimensions }',
      detail: 'raisin.ai.embed(request)',
    },
    {
      label: 'list_models',
      kind: CompletionItemKind.Method,
      insertText: 'list_models()',
      documentation: 'List available AI models\n\nReturns: list[{ id, name, provider, capabilities }]',
      detail: 'raisin.ai.list_models()',
    },
    {
      label: 'get_default_model',
      kind: CompletionItemKind.Method,
      insertText: 'get_default_model(${1:use_case})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Get default model for use case\n\nArgs:\n  use_case: "chat", "completion", "agent", "embedding"\n\nReturns: str | None',
      detail: 'raisin.ai.get_default_model(use_case)',
    },

    // ===== raisin.events - Event operations =====
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
      insertText: 'emit(${1:event_type}, ${2:data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Emit a custom event\n\nArgs:\n  event_type: Event type string\n  data: Event payload dict',
      detail: 'raisin.events.emit(event_type, data)',
    },

    // ===== raisin.tasks - Task operations =====
    {
      label: 'tasks',
      kind: CompletionItemKind.Property,
      insertText: 'tasks',
      documentation: 'Human task operations',
      detail: 'raisin.tasks',
    },
    {
      label: 'create',
      kind: CompletionItemKind.Method,
      insertText: 'create(${1:request})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Create a human task\n\nArgs:\n  request: { task_type, title, assignee, ... }\n\nReturns: { task_id, task_path }',
      detail: 'raisin.tasks.create(request)',
    },

    // ===== raisin.pdf - PDF operations =====
    {
      label: 'pdf',
      kind: CompletionItemKind.Property,
      insertText: 'pdf',
      documentation: 'PDF processing operations',
      detail: 'raisin.pdf',
    },
    {
      label: 'extract_text',
      kind: CompletionItemKind.Method,
      insertText: 'extract_text(${1:base64_data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Extract text from PDF\n\nArgs:\n  base64_data: Base64-encoded PDF\n\nReturns: { text, pages, page_count, is_scanned }',
      detail: 'raisin.pdf.extract_text(base64_data)',
    },
    {
      label: 'get_page_count',
      kind: CompletionItemKind.Method,
      insertText: 'get_page_count(${1:base64_data})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Get PDF page count\n\nArgs:\n  base64_data: Base64-encoded PDF\n\nReturns: int',
      detail: 'raisin.pdf.get_page_count(base64_data)',
    },
    {
      label: 'process_from_storage',
      kind: CompletionItemKind.Method,
      insertText: 'process_from_storage(${1:storage_key}, ${2:options})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Process PDF from storage (no base64 overhead)\n\nArgs:\n  storage_key: Storage key from resource metadata\n  options: { ocr, generate_thumbnail, ... }\n\nReturns: { text, page_count, is_scanned, thumbnail }',
      detail: 'raisin.pdf.process_from_storage(storage_key, options={})',
    },

    // ===== raisin.context - Execution context (read-only) =====
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
      detail: 'str',
    },
    {
      label: 'repo_id',
      kind: CompletionItemKind.Property,
      insertText: 'repo_id',
      documentation: 'Current repository ID',
      detail: 'str',
    },
    {
      label: 'branch',
      kind: CompletionItemKind.Property,
      insertText: 'branch',
      documentation: 'Current branch name',
      detail: 'str',
    },
    {
      label: 'workspace_id',
      kind: CompletionItemKind.Property,
      insertText: 'workspace_id',
      documentation: 'Current workspace ID',
      detail: 'str',
    },
    {
      label: 'actor',
      kind: CompletionItemKind.Property,
      insertText: 'actor',
      documentation: 'User/actor ID executing the function',
      detail: 'str',
    },
    {
      label: 'execution_id',
      kind: CompletionItemKind.Property,
      insertText: 'execution_id',
      documentation: 'Unique ID for this execution',
      detail: 'str',
    },

    // ===== Starlark builtins =====
    {
      label: 'print',
      kind: CompletionItemKind.Function,
      insertText: 'print(${1:message})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Print message (captured as log entry)',
      detail: 'print(message)',
    },
    {
      label: 'fail',
      kind: CompletionItemKind.Function,
      insertText: 'fail(${1:message})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Fail with error message (like raise Exception)',
      detail: 'fail(message)',
    },
    {
      label: 'struct',
      kind: CompletionItemKind.Function,
      insertText: 'struct(${1:})',
      insertTextRules: CompletionItemInsertTextRule.InsertAsSnippet,
      documentation: 'Create a struct (like a class instance)\n\nExample: struct(name="John", age=30)',
      detail: 'struct(**kwargs)',
    },
  ]
}

/**
 * Register RaisinDB Starlark/Python completion provider
 */
export function registerRaisinStarlarkCompletionProvider(monaco: Monaco): void {
  const completions = getRaisinStarlarkCompletions(monaco)

  monaco.languages.registerCompletionItemProvider(STARLARK_LANGUAGE_ID, {
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

      const suggestions: languages.CompletionItem[] = []

      // raisin.* completions
      if (/\braisin\.$/.test(textUntilPosition)) {
        // After "raisin." - show namespaces
        const namespaces = ['nodes', 'sql', 'http', 'ai', 'events', 'tasks', 'pdf', 'context']
        for (const ns of namespaces) {
          const completion = completions.find(
            (c) => c.label === ns && c.kind === monaco.languages.CompletionItemKind.Property
          )
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      } else if (/\braisin\.nodes\.$/.test(textUntilPosition)) {
        // After "raisin.nodes." - show node methods
        const methods = ['get', 'get_by_id', 'create', 'update', 'delete', 'query', 'get_children', 'update_property', 'move']
        for (const method of methods) {
          const completion = completions.find(
            (c) => c.label === method && c.kind === monaco.languages.CompletionItemKind.Method
          )
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      } else if (/\braisin\.sql\.$/.test(textUntilPosition)) {
        // After "raisin.sql." - show sql methods
        const methods = ['query', 'execute']
        for (const method of methods) {
          const completion = completions.find(
            (c) => c.label === method && c.kind === monaco.languages.CompletionItemKind.Method
          )
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      } else if (/\braisin\.http\.$/.test(textUntilPosition)) {
        // After "raisin.http." - show http methods
        const methods = ['get', 'post', 'put', 'patch', 'delete']
        for (const method of methods) {
          const completion = completions.find(
            (c) => c.label === method && c.kind === monaco.languages.CompletionItemKind.Method
          )
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      } else if (/\braisin\.ai\.$/.test(textUntilPosition)) {
        // After "raisin.ai." - show ai methods
        const methods = ['completion', 'embed', 'list_models', 'get_default_model']
        for (const method of methods) {
          const completion = completions.find(
            (c) => c.label === method && c.kind === monaco.languages.CompletionItemKind.Method
          )
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      } else if (/\braisin\.events\.$/.test(textUntilPosition)) {
        // After "raisin.events." - show events methods
        const completion = completions.find((c) => c.label === 'emit')
        if (completion) {
          suggestions.push({ ...completion, range })
        }
      } else if (/\braisin\.tasks\.$/.test(textUntilPosition)) {
        // After "raisin.tasks." - show tasks methods
        const completion = completions.find(
          (c) => c.label === 'create' && c.detail?.includes('tasks')
        )
        if (completion) {
          suggestions.push({ ...completion, range })
        }
      } else if (/\braisin\.pdf\.$/.test(textUntilPosition)) {
        // After "raisin.pdf." - show pdf methods
        const methods = ['extract_text', 'get_page_count', 'process_from_storage']
        for (const method of methods) {
          const completion = completions.find(
            (c) => c.label === method && c.kind === monaco.languages.CompletionItemKind.Method
          )
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      } else if (/\braisin\.context\.$/.test(textUntilPosition)) {
        // After "raisin.context." - show context properties
        const props = ['tenant_id', 'repo_id', 'branch', 'workspace_id', 'actor', 'execution_id']
        for (const prop of props) {
          const completion = completions.find(
            (c) => c.label === prop && c.kind === monaco.languages.CompletionItemKind.Property
          )
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      } else if (word.word === '' || /^[a-z]/i.test(word.word)) {
        // Global scope - show raisin and builtins
        const raisinCompletion = completions.find((c) => c.label === 'raisin')
        if (raisinCompletion) {
          suggestions.push({ ...raisinCompletion, range })
        }
        // Add Starlark builtins
        for (const builtin of ['print', 'fail', 'struct']) {
          const completion = completions.find((c) => c.label === builtin)
          if (completion) {
            suggestions.push({ ...completion, range })
          }
        }
      }

      return { suggestions }
    },
  })
}
