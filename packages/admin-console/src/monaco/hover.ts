/**
 * RaisinDB SQL hover provider for Monaco Editor
 *
 * Provides documentation on hover using the generated DDL keywords from Rust.
 */

import type { languages, editor, Position, CancellationToken } from 'monaco-editor'
import { ddlKeywords } from '../generated/ddl'
import type { KeywordInfo } from '../generated/ddl/KeywordInfo'
import { LANGUAGE_ID } from './language-config'

// Build a lookup map for fast keyword access
const keywordMap = new Map<string, KeywordInfo>()
for (const kw of ddlKeywords.keywords) {
  keywordMap.set(kw.keyword.toUpperCase(), kw)
}

// Additional documentation for standard SQL keywords
const standardSqlDocs = new Map<string, { description: string; syntax?: string; example?: string }>([
  ['SELECT', {
    description: 'Retrieve rows and columns from one or more tables.',
    syntax: 'SELECT [DISTINCT] columns FROM table [WHERE condition] [ORDER BY column] [LIMIT n]',
    example: "SELECT name, type FROM nodes WHERE type = 'Article' ORDER BY created_at DESC LIMIT 10",
  }],
  ['FROM', {
    description: 'Specifies the source table(s) for the query.',
    syntax: 'FROM table_name [alias] [, table_name [alias]]',
    example: 'FROM nodes n, relations r',
  }],
  ['WHERE', {
    description: 'Filters rows based on specified conditions.',
    syntax: 'WHERE condition [AND|OR condition]',
    example: "WHERE status = 'published' AND created_at > '2024-01-01'",
  }],
  ['JOIN', {
    description: 'Combines rows from two or more tables based on a related column.',
    syntax: '[LEFT|RIGHT|INNER|CROSS] JOIN table ON condition',
    example: 'JOIN children c ON parent.id = c.parent_id',
  }],
  ['GROUP BY', {
    description: 'Groups rows that have the same values into summary rows.',
    syntax: 'GROUP BY column1 [, column2, ...]',
    example: 'SELECT type, COUNT(*) FROM nodes GROUP BY type',
  }],
  ['ORDER BY', {
    description: 'Sorts the result set by one or more columns.',
    syntax: 'ORDER BY column [ASC|DESC] [, column [ASC|DESC]]',
    example: 'ORDER BY created_at DESC, name ASC',
  }],
  ['LIMIT', {
    description: 'Constrains the number of rows returned.',
    syntax: 'LIMIT count [OFFSET skip]',
    example: 'LIMIT 10 OFFSET 20',
  }],
  ['DISTINCT', {
    description: 'Returns only unique rows, removing duplicates.',
    syntax: 'SELECT DISTINCT columns',
    example: 'SELECT DISTINCT type FROM nodes',
  }],
  ['AS', {
    description: 'Creates an alias for a column or table.',
    syntax: 'expression AS alias',
    example: 'SELECT COUNT(*) AS total, type AS node_type FROM nodes',
  }],
  ['CASE', {
    description: 'Conditional expression that returns different values based on conditions.',
    syntax: 'CASE WHEN condition THEN result [WHEN ...] [ELSE default] END',
    example: "CASE WHEN status = 'active' THEN 'Yes' ELSE 'No' END AS is_active",
  }],
  ['WITH', {
    description: 'Defines a Common Table Expression (CTE) for use in the query.',
    syntax: 'WITH cte_name AS (subquery) SELECT ... FROM cte_name',
    example: 'WITH recent AS (SELECT * FROM nodes ORDER BY created_at DESC LIMIT 100) SELECT type, COUNT(*) FROM recent GROUP BY type',
  }],
  ['EXPLAIN', {
    description: 'Shows the execution plan for a query without executing it.',
    syntax: 'EXPLAIN [ANALYZE] query',
    example: 'EXPLAIN SELECT * FROM nodes WHERE type = \'Article\'',
  }],
  ['ANALYZE', {
    description: 'When used with EXPLAIN, executes the query and shows actual execution statistics.',
    syntax: 'EXPLAIN ANALYZE query',
    example: 'EXPLAIN ANALYZE SELECT * FROM nodes WHERE parent_id IS NOT NULL',
  }],
  ['OVER', {
    description: 'Defines a window for window functions.',
    syntax: 'function() OVER ([PARTITION BY col] [ORDER BY col] [frame])',
    example: 'ROW_NUMBER() OVER (PARTITION BY type ORDER BY created_at DESC)',
  }],
  ['PARTITION BY', {
    description: 'Divides the result set into partitions for window functions.',
    syntax: 'PARTITION BY column1 [, column2, ...]',
    example: 'RANK() OVER (PARTITION BY category ORDER BY score DESC)',
  }],
  // Transaction keywords
  ['BEGIN', {
    description: 'Starts a transaction block. Multiple statements can be executed atomically within a transaction.',
    syntax: 'BEGIN [TRANSACTION]; statements; COMMIT;',
    example: "BEGIN; UPDATE workspace SET name = 'new'; COMMIT WITH MESSAGE 'Updated name';",
  }],
  ['COMMIT', {
    description: 'Commits the current transaction, saving all changes. Supports optional message and actor.',
    syntax: "COMMIT [WITH MESSAGE 'message' [ACTOR 'actor']]",
    example: "COMMIT WITH MESSAGE 'Updated user profile' ACTOR 'admin@example.com'",
  }],
  ['ROLLBACK', {
    description: 'Rolls back the current transaction, undoing all changes since BEGIN.',
    syntax: 'ROLLBACK',
    example: 'ROLLBACK;',
  }],
  ['TRANSACTION', {
    description: 'Optional keyword after BEGIN to explicitly start a transaction.',
    syntax: 'BEGIN TRANSACTION',
    example: 'BEGIN TRANSACTION;',
  }],
  // Tree manipulation keywords
  ['ORDER', {
    description: 'Reorders sibling nodes within their parent using ABOVE or BELOW positioning.',
    syntax: "ORDER workspace SET path='/node' ABOVE|BELOW path='/sibling'",
    example: "ORDER default SET path='/content/post3' ABOVE path='/content/post1';",
  }],
  ['MOVE', {
    description: 'Moves a node and its entire subtree to a new parent location. Node IDs are preserved.',
    syntax: "MOVE workspace SET path='/source' TO path='/new-parent'",
    example: "MOVE default SET path='/content/draft' TO path='/content/published';",
  }],
  ['ABOVE', {
    description: 'Positions a node before (above) a sibling in the ORDER statement.',
    syntax: "ORDER workspace SET path='/node' ABOVE path='/sibling'",
    example: "ORDER default SET path='/users/carol' ABOVE path='/users/alice';",
  }],
  ['BELOW', {
    description: 'Positions a node after (below) a sibling in the ORDER statement.',
    syntax: "ORDER workspace SET path='/node' BELOW path='/sibling'",
    example: "ORDER default SET path='/users/alice' BELOW path='/users/carol';",
  }],
  ['TO', {
    description: 'Specifies the target parent in MOVE statements.',
    syntax: "MOVE workspace SET path='/source' TO path='/target-parent'",
    example: "MOVE default SET path='/old/node' TO path='/new/parent';",
  }],
])

export function createHoverProvider(
  _monaco: typeof import('monaco-editor')
): languages.HoverProvider {
  return {
    provideHover(
      model: editor.ITextModel,
      position: Position,
      _token: CancellationToken
    ): languages.ProviderResult<languages.Hover> {
      const word = model.getWordAtPosition(position)
      if (!word) {
        return null
      }

      const wordUpperCase = word.word.toUpperCase()

      // Check DDL keywords first
      const ddlKeyword = keywordMap.get(wordUpperCase)
      if (ddlKeyword) {
        const contents: string[] = [
          `**${ddlKeyword.keyword}** \`[${ddlKeyword.category}]\``,
          '',
          ddlKeyword.description,
        ]

        if (ddlKeyword.syntax) {
          contents.push('', '**Syntax:**', '```sql', ddlKeyword.syntax, '```')
        }

        if (ddlKeyword.example) {
          contents.push('', '**Example:**', '```sql', ddlKeyword.example, '```')
        }

        return {
          contents: [{ value: contents.join('\n') }],
          range: {
            startLineNumber: position.lineNumber,
            endLineNumber: position.lineNumber,
            startColumn: word.startColumn,
            endColumn: word.endColumn,
          },
        }
      }

      // Check standard SQL keywords
      const sqlDoc = standardSqlDocs.get(wordUpperCase)
      if (sqlDoc) {
        const contents: string[] = [
          `**${wordUpperCase}** \`[SQL]\``,
          '',
          sqlDoc.description,
        ]

        if (sqlDoc.syntax) {
          contents.push('', '**Syntax:**', '```sql', sqlDoc.syntax, '```')
        }

        if (sqlDoc.example) {
          contents.push('', '**Example:**', '```sql', sqlDoc.example, '```')
        }

        return {
          contents: [{ value: contents.join('\n') }],
          range: {
            startLineNumber: position.lineNumber,
            endLineNumber: position.lineNumber,
            startColumn: word.startColumn,
            endColumn: word.endColumn,
          },
        }
      }

      // Check for namespace:type patterns
      const lineContent = model.getLineContent(position.lineNumber)
      const colonMatch = lineContent.match(/(['"]?)([a-zA-Z_][\w]*):([a-zA-Z_][\w]*)\1/g)
      if (colonMatch) {
        for (const match of colonMatch) {
          const cleanMatch = match.replace(/['"]/g, '')
          const startIndex = lineContent.indexOf(match)
          const endIndex = startIndex + match.length

          if (position.column > startIndex && position.column <= endIndex + 1) {
            return {
              contents: [{
                value: [
                  `**${cleanMatch}** \`[Type Reference]\``,
                  '',
                  'A namespaced type identifier in the format `namespace:TypeName`.',
                  '',
                  '- **Namespace**: Identifies the package or module (e.g., `cms`, `myapp`)',
                  '- **TypeName**: The specific type name (e.g., `Article`, `Page`)',
                ].join('\n'),
              }],
              range: {
                startLineNumber: position.lineNumber,
                endLineNumber: position.lineNumber,
                startColumn: startIndex + 1,
                endColumn: endIndex + 1,
              },
            }
          }
        }
      }

      return null
    },
  }
}

export function registerHoverProvider(
  monaco: typeof import('monaco-editor')
): void {
  monaco.languages.registerHoverProvider(LANGUAGE_ID, createHoverProvider(monaco))
}
