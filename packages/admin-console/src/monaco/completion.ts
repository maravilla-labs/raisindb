/**
 * RaisinDB SQL completion provider for Monaco Editor
 *
 * Provides intelligent autocomplete suggestions using:
 * - WASM-based semantic completions for context-aware suggestions
 * - DDL keywords from generated Rust data
 * - Schema cache for table/column completions
 */

import type { languages, editor, Position, CancellationToken } from 'monaco-editor'
import { ddlKeywords } from '../generated/ddl'
import type { KeywordCategory } from '../generated/ddl/KeywordCategory'
import { LANGUAGE_ID } from './language-config'
import { getSchemaCache } from './schema-cache'
import type { CompletionItem as WasmCompletionItem, CompletionResult } from './validation/types'

// Map keyword categories to Monaco completion item kinds
function categoryToCompletionKind(
  category: KeywordCategory,
  kinds: typeof languages.CompletionItemKind
): languages.CompletionItemKind {
  switch (category) {
    case 'Statement':
      return kinds.Keyword
    case 'SchemaObject':
      return kinds.Class
    case 'Clause':
      return kinds.Keyword
    case 'PropertyType':
      return kinds.TypeParameter
    case 'Modifier':
      return kinds.Property
    case 'Flag':
      return kinds.Constant
    case 'Operator':
      return kinds.Operator
    case 'SqlFunction':
    case 'JsonFunction':
    case 'TableFunction':
    case 'AggregateFunction':
    case 'WindowFunction':
      return kinds.Function
    default:
      return kinds.Text
  }
}

// Get sort priority for category (lower = higher priority)
function categorySortPriority(category: KeywordCategory): string {
  switch (category) {
    case 'Statement':
      return '0'
    case 'SchemaObject':
      return '1'
    case 'PropertyType':
      return '2'
    case 'Modifier':
      return '3'
    case 'Clause':
      return '4'
    case 'Flag':
      return '5'
    case 'SqlFunction':
    case 'JsonFunction':
    case 'TableFunction':
    case 'AggregateFunction':
    case 'WindowFunction':
      return '6'
    case 'Operator':
      return '7'
    default:
      return '9'
  }
}

// Standard SQL keywords for completion
const standardSqlKeywords = [
  { keyword: 'SELECT', description: 'Query rows from tables', syntax: 'SELECT columns FROM table', example: 'SELECT * FROM nodes' },
  { keyword: 'FROM', description: 'Specify the source table(s)', syntax: 'FROM table_name [alias]', example: 'SELECT * FROM nodes n' },
  { keyword: 'WHERE', description: 'Filter rows based on conditions', syntax: 'WHERE condition', example: "WHERE type = 'Article'" },
  { keyword: 'AND', description: 'Logical AND operator', syntax: 'condition1 AND condition2', example: "WHERE a = 1 AND b = 2" },
  { keyword: 'OR', description: 'Logical OR operator', syntax: 'condition1 OR condition2', example: "WHERE a = 1 OR a = 2" },
  { keyword: 'NOT', description: 'Logical NOT operator', syntax: 'NOT condition', example: "WHERE NOT deleted" },
  { keyword: 'JOIN', description: 'Join two tables', syntax: 'JOIN table ON condition', example: 'JOIN children c ON n.id = c.parent_id' },
  { keyword: 'LEFT JOIN', description: 'Left outer join', syntax: 'LEFT JOIN table ON condition', example: 'LEFT JOIN parent p ON n.parent_id = p.id' },
  { keyword: 'GROUP BY', description: 'Group rows for aggregation', syntax: 'GROUP BY column1, column2', example: 'GROUP BY type' },
  { keyword: 'ORDER BY', description: 'Sort the result set', syntax: 'ORDER BY column [ASC|DESC]', example: 'ORDER BY created_at DESC' },
  { keyword: 'LIMIT', description: 'Limit the number of rows returned', syntax: 'LIMIT count', example: 'LIMIT 10' },
  { keyword: 'OFFSET', description: 'Skip rows before returning results', syntax: 'OFFSET count', example: 'LIMIT 10 OFFSET 20' },
  { keyword: 'AS', description: 'Alias for column or table', syntax: 'expression AS alias', example: 'SELECT COUNT(*) AS total' },
  { keyword: 'DISTINCT', description: 'Remove duplicate rows', syntax: 'SELECT DISTINCT columns', example: 'SELECT DISTINCT type' },
  { keyword: 'IN', description: 'Match against a list of values', syntax: "column IN (value1, value2)", example: "WHERE status IN ('active', 'pending')" },
  { keyword: 'LIKE', description: 'Pattern matching with wildcards', syntax: "column LIKE 'pattern%'", example: "WHERE name LIKE 'Article%'" },
  { keyword: 'IS NULL', description: 'Check for NULL values', syntax: 'column IS NULL', example: 'WHERE deleted_at IS NULL' },
  { keyword: 'IS NOT NULL', description: 'Check for non-NULL values', syntax: 'column IS NOT NULL', example: 'WHERE parent_id IS NOT NULL' },
  { keyword: 'CASE', description: 'Conditional expression', syntax: 'CASE WHEN condition THEN result ELSE default END', example: "CASE WHEN status = 'active' THEN 1 ELSE 0 END" },
  { keyword: 'EXPLAIN', description: 'Show query execution plan', syntax: 'EXPLAIN query', example: 'EXPLAIN SELECT * FROM nodes' },
  { keyword: 'EXPLAIN ANALYZE', description: 'Execute and show actual query plan', syntax: 'EXPLAIN ANALYZE query', example: 'EXPLAIN ANALYZE SELECT * FROM nodes' },
  { keyword: 'WITH', description: 'Common Table Expression (CTE)', syntax: 'WITH name AS (subquery) SELECT ...', example: 'WITH recent AS (SELECT * FROM nodes ORDER BY created_at DESC LIMIT 10) SELECT * FROM recent' },
  // DML keywords
  { keyword: 'INSERT', description: 'Insert new nodes into workspace', syntax: "INSERT INTO workspace ('/path') VALUES (...)", example: "INSERT INTO content ('/articles/new') VALUES ('title', 'body')" },
  { keyword: 'UPSERT', description: 'Insert or update nodes (create if not exists, update if exists)', syntax: "UPSERT INTO workspace ('/path') VALUES (...)", example: "UPSERT INTO content ('/articles/new') VALUES ('title', 'body')" },
  { keyword: 'UPDATE', description: 'Update existing nodes in workspace', syntax: "UPDATE workspace SET column = value WHERE condition", example: "UPDATE content SET title = 'New Title' WHERE path = '/articles/post1'" },
  { keyword: 'DELETE', description: 'Delete nodes from workspace', syntax: "DELETE FROM workspace WHERE condition", example: "DELETE FROM content WHERE path = '/articles/old'" },
  // Transaction keywords
  { keyword: 'BEGIN', description: 'Start a transaction block', syntax: 'BEGIN; statements; COMMIT;', example: "BEGIN; UPDATE workspace SET name = 'new'; COMMIT;" },
  { keyword: 'COMMIT', description: 'Commit transaction with optional message', syntax: "COMMIT [WITH MESSAGE 'msg' ACTOR 'actor']", example: "COMMIT WITH MESSAGE 'Updated user' ACTOR 'admin'" },
  { keyword: 'ROLLBACK', description: 'Rollback current transaction', syntax: 'ROLLBACK;', example: 'ROLLBACK;' },
  { keyword: 'TRANSACTION', description: 'Transaction keyword (optional)', syntax: 'BEGIN TRANSACTION', example: 'BEGIN TRANSACTION;' },
  // Tree manipulation keywords
  { keyword: 'ORDER', description: 'Reorder sibling nodes (ABOVE/BELOW)', syntax: "ORDER workspace SET path='/node' ABOVE|BELOW path='/sibling'", example: "ORDER default SET path='/content/post3' ABOVE path='/content/post1'" },
  { keyword: 'MOVE', description: 'Move node subtree to new parent', syntax: "MOVE workspace SET path='/source' TO path='/new-parent'", example: "MOVE default SET path='/content/draft' TO path='/content/published'" },
  { keyword: 'ABOVE', description: 'Position node before sibling (ORDER)', syntax: "ORDER workspace SET path='/node' ABOVE path='/sibling'", example: "ORDER default SET path='/a' ABOVE path='/b'" },
  { keyword: 'BELOW', description: 'Position node after sibling (ORDER)', syntax: "ORDER workspace SET path='/node' BELOW path='/sibling'", example: "ORDER default SET path='/a' BELOW path='/b'" },
  // Reference query functions
  { keyword: 'REFERENCES', description: 'Query nodes that reference a specific target path', syntax: "REFERENCES('workspace:/path')", example: "WHERE REFERENCES('social:/demonews/tags/tech-stack/rust')" },
  { keyword: 'DESCENDANT_OF', description: 'Filter nodes that are descendants of a path', syntax: "DESCENDANT_OF('/parent/path')", example: "WHERE DESCENDANT_OF('/demonews/articles')" },
  { keyword: 'CHILD_OF', description: 'Filter nodes that are direct children of a path', syntax: "CHILD_OF('/parent/path')", example: "WHERE CHILD_OF('/demonews/articles/tech')" },
]

// Context detection helpers
function getTextBeforeCursor(model: editor.ITextModel, position: Position): string {
  const lineContent = model.getLineContent(position.lineNumber)
  return lineContent.substring(0, position.column - 1).toUpperCase()
}

function getRecentContext(model: editor.ITextModel, position: Position): string {
  // Get last 5 lines for context
  const startLine = Math.max(1, position.lineNumber - 4)
  let context = ''
  for (let i = startLine; i <= position.lineNumber; i++) {
    context += model.getLineContent(i) + ' '
  }
  return context.toUpperCase()
}

export function createCompletionProvider(
  monaco: typeof import('monaco-editor')
): languages.CompletionItemProvider {
  return {
    triggerCharacters: [' ', '.', '(', "'"],

    provideCompletionItems(
      model: editor.ITextModel,
      position: Position,
      _context: languages.CompletionContext,
      _token: CancellationToken
    ): languages.ProviderResult<languages.CompletionList> {
      const textBeforeCursor = getTextBeforeCursor(model, position)
      const recentContext = getRecentContext(model, position)
      const word = model.getWordUntilPosition(position)

      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      }

      const suggestions: languages.CompletionItem[] = []

      // Detect context for filtering
      const isAfterCreate = /\bCREATE\s*$/.test(textBeforeCursor)
      const isAfterAlter = /\bALTER\s*$/.test(textBeforeCursor)
      const isAfterDrop = /\bDROP\s*$/.test(textBeforeCursor)
      const isInDDL = /\b(CREATE|ALTER|DROP|MERGE|USE|CHECKOUT|SHOW|DESCRIBE)\s+(NODETYPE|ARCHETYPE|ELEMENTTYPE|BRANCH|BRANCHES|CURRENT|DIVERGENCE)\b/.test(recentContext)
      const isAfterProperty = /\bPROPERT(Y|IES)\s*\(\s*[^)]*$/.test(recentContext) ||
                              /\bADD\s+PROPERTY\s+\w+\s*$/.test(textBeforeCursor)
      const isExpectingType = /\b(PROPERTY\s+\w+|,\s*\w+)\s*$/.test(textBeforeCursor)

      // Add DDL keywords from generated data
      for (const kw of ddlKeywords.keywords) {
        // Context-aware filtering
        if (isAfterCreate || isAfterAlter || isAfterDrop) {
          // Only show schema objects after CREATE/ALTER/DROP
          if (kw.category !== 'SchemaObject') continue
        } else if (isExpectingType) {
          // Show property types when expecting a type
          if (kw.category !== 'PropertyType') continue
        } else if (isAfterProperty && kw.category === 'Statement') {
          // Don't show statement keywords inside property definitions
          continue
        }

        suggestions.push({
          label: kw.keyword,
          kind: categoryToCompletionKind(kw.category, monaco.languages.CompletionItemKind),
          insertText: kw.keyword,
          detail: `[${kw.category}] ${kw.description}`,
          documentation: {
            value: [
              kw.description,
              kw.syntax ? `\n\n**Syntax:** \`${kw.syntax}\`` : '',
              kw.example ? `\n\n**Example:**\n\`\`\`sql\n${kw.example}\n\`\`\`` : '',
            ].join(''),
          },
          sortText: categorySortPriority(kw.category) + kw.keyword,
          range,
        })
      }

      // Add standard SQL keywords (unless in DDL context expecting specific items)
      if (!isAfterCreate && !isAfterAlter && !isAfterDrop && !isExpectingType) {
        for (const kw of standardSqlKeywords) {
          suggestions.push({
            label: kw.keyword,
            kind: monaco.languages.CompletionItemKind.Keyword,
            insertText: kw.keyword,
            detail: kw.description,
            documentation: {
              value: [
                kw.description,
                kw.syntax ? `\n\n**Syntax:** \`${kw.syntax}\`` : '',
                kw.example ? `\n\n**Example:**\n\`\`\`sql\n${kw.example}\n\`\`\`` : '',
              ].join(''),
            },
            sortText: '8' + kw.keyword, // Lower priority than DDL keywords
            range,
          })
        }
      }

      // Add snippet suggestions for common patterns
      if (!isInDDL) {
        suggestions.push({
          label: 'select-all',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: 'SELECT * FROM ${1:nodes} WHERE ${2:condition} LIMIT ${3:10}',
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Select all columns with filter',
          documentation: 'Insert a basic SELECT query template',
          sortText: 'z0',
          range,
        })

        suggestions.push({
          label: 'select-children',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "SELECT * FROM CHILDREN('${1:parent-id}')",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Select child nodes',
          documentation: 'Query direct children of a node',
          sortText: 'z1',
          range,
        })

        suggestions.push({
          label: 'select-descendants',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "SELECT * FROM DESCENDANTS('${1:ancestor-id}')",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Select all descendants',
          documentation: 'Query all descendants of a node recursively',
          sortText: 'z2',
          range,
        })

        suggestions.push({
          label: 'explain-analyze',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: 'EXPLAIN ANALYZE ${1:SELECT * FROM nodes}',
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Analyze query execution plan',
          documentation: 'Run query with execution plan analysis',
          sortText: 'z3',
          range,
        })

        suggestions.push({
          label: 'select-references',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "SELECT * FROM ${1:workspace} WHERE REFERENCES('${2:workspace}:${3:/path/to/target}')",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Find nodes referencing a target',
          documentation: 'Query all nodes that have a reference to the specified target path',
          sortText: 'z4',
          range,
        })
      }

      // DDL snippets
      if (textBeforeCursor.trim() === '' || /^\s*$/.test(textBeforeCursor)) {
        suggestions.push({
          label: 'create-nodetype',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "CREATE NODETYPE '${1:namespace}:${2:TypeName}'",
            "  EXTENDS '${3:raisin:Content}'",
            '  PROPERTIES (',
            '    ${4:title} String REQUIRED,',
            '    ${5:description} String',
            '  )',
          ].join('\n'),
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Create a new node type',
          documentation: 'Insert a CREATE NODETYPE template with properties',
          sortText: 'z4',
          range,
        })

        suggestions.push({
          label: 'create-archetype',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "CREATE ARCHETYPE '${1:archetype-name}'",
            "  BASE_NODE_TYPE '${2:namespace}:${3:TypeName}'",
            "  TITLE '${4:Display Title}'",
            '  FIELDS (',
            '    ${5:field_name}',
            '  )',
          ].join('\n'),
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Create a new archetype',
          documentation: 'Insert a CREATE ARCHETYPE template',
          sortText: 'z5',
          range,
        })

        suggestions.push({
          label: 'create-branch',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "CREATE BRANCH '${1:feature/new-branch}' FROM '${2:main}'",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Create a new branch',
          documentation: 'Insert a CREATE BRANCH statement',
          sortText: 'z6',
          range,
        })

        suggestions.push({
          label: 'merge-branch',
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "MERGE BRANCH '${1:source-branch}' INTO '${2:main}'",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: 'Merge a branch',
          documentation: 'Insert a MERGE BRANCH statement',
          sortText: 'z7',
          range,
        })
      }

      return { suggestions }
    },
  }
}

export function registerCompletionProvider(
  monaco: typeof import('monaco-editor')
): void {
  monaco.languages.registerCompletionItemProvider(
    LANGUAGE_ID,
    createCompletionProvider(monaco)
  )
}

// =============================================================================
// Semantic Completion Provider (WASM-based)
// =============================================================================

/**
 * Map WASM completion kind to Monaco completion kind
 */
function wasmKindToMonacoKind(
  kind: string,
  kinds: typeof languages.CompletionItemKind
): languages.CompletionItemKind {
  switch (kind) {
    case 'keyword':
      return kinds.Keyword
    case 'table':
      return kinds.Class
    case 'column':
      return kinds.Field
    case 'function':
      return kinds.Function
    case 'aggregate':
      return kinds.Function
    case 'snippet':
      return kinds.Snippet
    case 'type':
      return kinds.TypeParameter
    case 'alias':
      return kinds.Variable
    case 'operator':
      return kinds.Operator
    default:
      return kinds.Text
  }
}

/**
 * Convert WASM completion item to Monaco completion item
 */
function wasmItemToMonacoItem(
  item: WasmCompletionItem,
  monaco: typeof import('monaco-editor'),
  range: { startLineNumber: number; endLineNumber: number; startColumn: number; endColumn: number }
): languages.CompletionItem {
  return {
    label: item.label,
    kind: wasmKindToMonacoKind(item.kind, monaco.languages.CompletionItemKind),
    insertText: item.insert_text,
    insertTextRules:
      item.insert_text_format === 'snippet'
        ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
        : undefined,
    detail: item.detail,
    documentation: item.documentation
      ? { value: item.documentation }
      : undefined,
    sortText: item.sort_text ?? item.label,
    filterText: item.filter_text ?? item.label,
    range,
  }
}

/**
 * Get completions from WASM via worker
 */
type CompletionGetter = (sql: string, offset: number) => Promise<CompletionResult | null>

/**
 * Create a semantic completion provider that uses WASM for context-aware completions
 *
 * @param monaco - Monaco editor module
 * @param getCompletions - Function to get completions from WASM worker
 */
export function createSemanticCompletionProvider(
  monaco: typeof import('monaco-editor'),
  getCompletions: CompletionGetter
): languages.CompletionItemProvider {
  return {
    triggerCharacters: [' ', '.', '(', ',', "'"],

    async provideCompletionItems(
      model: editor.ITextModel,
      position: Position,
      _context: languages.CompletionContext,
      _token: CancellationToken
    ): Promise<languages.CompletionList> {
      const word = model.getWordUntilPosition(position)
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      }

      const suggestions: languages.CompletionItem[] = []

      try {
        // Get SQL text and cursor offset
        const sql = model.getValue()
        const offset = model.getOffsetAt(position)

        // Call WASM for semantic completions
        const result = await getCompletions(sql, offset)

        if (result && result.items.length > 0) {
          // Use WASM completions
          for (const item of result.items) {
            suggestions.push(wasmItemToMonacoItem(item, monaco, range))
          }
        } else {
          // Fall back to keyword-based completions
          addFallbackCompletions(suggestions, monaco, model, position, range)
        }
      } catch (error) {
        console.error('[Completion] WASM error, falling back to keywords:', error)
        // Fall back to keyword-based completions on error
        addFallbackCompletions(suggestions, monaco, model, position, range)
      }

      return { suggestions }
    },
  }
}

/**
 * Add fallback keyword-based completions when WASM is unavailable
 */
function addFallbackCompletions(
  suggestions: languages.CompletionItem[],
  monaco: typeof import('monaco-editor'),
  model: editor.ITextModel,
  position: Position,
  range: { startLineNumber: number; endLineNumber: number; startColumn: number; endColumn: number }
) {
  const textBeforeCursor = getTextBeforeCursor(model, position)
  // recentContext is available if needed for more context detection
  // const recentContext = getRecentContext(model, position)

  // Context detection
  const isAfterCreate = /\bCREATE\s*$/i.test(textBeforeCursor)
  const isAfterAlter = /\bALTER\s*$/i.test(textBeforeCursor)
  const isAfterDrop = /\bDROP\s*$/i.test(textBeforeCursor)
  const isAfterFrom = /\bFROM\s*$/i.test(textBeforeCursor) || /\bJOIN\s*$/i.test(textBeforeCursor)
  const isAfterDot = /\w+\.\s*$/i.test(textBeforeCursor)

  // Table suggestions after FROM/JOIN
  if (isAfterFrom) {
    const cache = getSchemaCache()
    for (const table of cache.getAllTables()) {
      suggestions.push({
        label: table.displayName,
        kind: monaco.languages.CompletionItemKind.Class,
        insertText: table.displayName,
        detail: table.isWorkspace ? 'Workspace' : 'Table',
        sortText: '0' + table.displayName,
        range,
      })
    }
    return
  }

  // Column suggestions after table.
  if (isAfterDot) {
    const match = textBeforeCursor.match(/(\w+)\.\s*$/i)
    if (match) {
      const tableName = match[1]
      const cache = getSchemaCache()
      const columns = cache.getColumnsForTable(tableName)
      for (const col of columns) {
        suggestions.push({
          label: col.name,
          kind: monaco.languages.CompletionItemKind.Field,
          insertText: col.name,
          detail: `${col.dataType}${col.nullable ? ' (nullable)' : ''}`,
          sortText: '0' + col.name,
          range,
        })
      }
    }
    return
  }

  // DDL object suggestions after CREATE/ALTER/DROP
  if (isAfterCreate || isAfterAlter || isAfterDrop) {
    for (const kw of ddlKeywords.keywords) {
      if (kw.category === 'SchemaObject') {
        suggestions.push({
          label: kw.keyword,
          kind: monaco.languages.CompletionItemKind.Class,
          insertText: kw.keyword,
          detail: kw.description,
          sortText: '0' + kw.keyword,
          range,
        })
      }
    }
    return
  }

  // Default: show all keywords
  for (const kw of ddlKeywords.keywords) {
    suggestions.push({
      label: kw.keyword,
      kind: categoryToCompletionKind(kw.category, monaco.languages.CompletionItemKind),
      insertText: kw.keyword,
      detail: `[${kw.category}] ${kw.description}`,
      sortText: categorySortPriority(kw.category) + kw.keyword,
      range,
    })
  }

  // Standard SQL keywords
  for (const kw of standardSqlKeywords) {
    suggestions.push({
      label: kw.keyword,
      kind: monaco.languages.CompletionItemKind.Keyword,
      insertText: kw.keyword,
      detail: kw.description,
      sortText: '8' + kw.keyword,
      range,
    })
  }
}

/**
 * Register the semantic completion provider
 */
export function registerSemanticCompletionProvider(
  monaco: typeof import('monaco-editor'),
  getCompletions: CompletionGetter
): void {
  monaco.languages.registerCompletionItemProvider(
    LANGUAGE_ID,
    createSemanticCompletionProvider(monaco, getCompletions)
  )
}
