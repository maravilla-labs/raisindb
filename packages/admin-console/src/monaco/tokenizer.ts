/**
 * RaisinDB SQL Monarch tokenizer for syntax highlighting
 *
 * Uses the generated DDL keywords from Rust to provide
 * comprehensive syntax highlighting for RaisinDB-specific SQL.
 */

import type { languages } from 'monaco-editor'
import { ddlKeywords } from '../generated/ddl'
import type { KeywordCategory } from '../generated/ddl/KeywordCategory'

// Build keyword lists by category from the generated data
function getKeywordsByCategory(category: KeywordCategory): string[] {
  return ddlKeywords.keywords
    .filter((kw) => kw.category === category)
    .map((kw) => kw.keyword)
}

// DDL statements
const statementKeywords = getKeywordsByCategory('Statement')
// Schema objects
const schemaObjectKeywords = getKeywordsByCategory('SchemaObject')
// Clauses
const clauseKeywords = getKeywordsByCategory('Clause')
// Property types
const propertyTypeKeywords = getKeywordsByCategory('PropertyType')
// Modifiers
const modifierKeywords = getKeywordsByCategory('Modifier')
// Flags
const flagKeywords = getKeywordsByCategory('Flag')
// Operators
const operatorKeywords = getKeywordsByCategory('Operator')
// SQL Functions
const sqlFunctionKeywords = getKeywordsByCategory('SqlFunction')
// JSON Functions
const jsonFunctionKeywords = getKeywordsByCategory('JsonFunction')
// Table Functions
const tableFunctionKeywords = getKeywordsByCategory('TableFunction')
// Aggregate Functions
const aggregateFunctionKeywords = getKeywordsByCategory('AggregateFunction')
// Window Functions
const windowFunctionKeywords = getKeywordsByCategory('WindowFunction')

// All functions combined
const allFunctions = [
  ...sqlFunctionKeywords,
  ...jsonFunctionKeywords,
  ...tableFunctionKeywords,
  ...aggregateFunctionKeywords,
  ...windowFunctionKeywords,
]

// Cypher keywords for embedded CYPHER() function calls
const cypherKeywords = [
  'MATCH',
  'OPTIONAL',
  'WHERE',
  'RETURN',
  'WITH',
  'UNWIND',
  'ORDER',
  'BY',
  'SKIP',
  'LIMIT',
  'CREATE',
  'MERGE',
  'DELETE',
  'DETACH',
  'SET',
  'REMOVE',
  'CALL',
  'YIELD',
  'UNION',
  'ALL',
  'AS',
  'DISTINCT',
  'CASE',
  'WHEN',
  'THEN',
  'ELSE',
  'END',
  'AND',
  'OR',
  'XOR',
  'NOT',
  'IN',
  'STARTS',
  'ENDS',
  'CONTAINS',
  'IS',
  'NULL',
  'TRUE',
  'FALSE',
  'EXISTS',
  'NONE',
  'ANY',
  'SINGLE',
  'FOREACH',
  'ON',
  'ASC',
  'DESC',
  'ASCENDING',
  'DESCENDING',
]

// Cypher functions
const cypherFunctions = [
  'id',
  'type',
  'labels',
  'keys',
  'properties',
  'nodes',
  'relationships',
  'length',
  'size',
  'head',
  'last',
  'tail',
  'range',
  'reverse',
  'toString',
  'toInteger',
  'toFloat',
  'toBoolean',
  'trim',
  'ltrim',
  'rtrim',
  'replace',
  'substring',
  'left',
  'right',
  'split',
  'toLower',
  'toUpper',
  'startNode',
  'endNode',
  'coalesce',
  'timestamp',
  'date',
  'datetime',
  'time',
  'duration',
  'point',
  'distance',
  'abs',
  'ceil',
  'floor',
  'round',
  'sign',
  'rand',
  'log',
  'log10',
  'exp',
  'sqrt',
  'sin',
  'cos',
  'tan',
  'asin',
  'acos',
  'atan',
  'atan2',
  'degrees',
  'radians',
  'pi',
  'e',
  'shortestPath',
  'allShortestPaths',
  'count',
  'sum',
  'avg',
  'min',
  'max',
  'collect',
]

// Standard SQL keywords that should be highlighted
const standardSqlKeywords = [
  'SELECT',
  'FROM',
  'WHERE',
  'AND',
  'OR',
  'NOT',
  'IN',
  'EXISTS',
  'BETWEEN',
  'LIKE',
  'IS',
  'NULL',
  'TRUE',
  'FALSE',
  'AS',
  'JOIN',
  'LEFT',
  'RIGHT',
  'INNER',
  'OUTER',
  'CROSS',
  'ON',
  'GROUP',
  'BY',
  'HAVING',
  'ORDER',
  'ASC',
  'DESC',
  'LIMIT',
  'OFFSET',
  'UNION',
  'ALL',
  'DISTINCT',
  'CASE',
  'WHEN',
  'THEN',
  'ELSE',
  'END',
  'INSERT',
  'UPSERT',
  'UPDATE',
  'DELETE',
  'INTO',
  'VALUES',
  'SET',
  'WITH',
  'OVER',
  'PARTITION',
  'ROWS',
  'RANGE',
  'UNBOUNDED',
  'PRECEDING',
  'FOLLOWING',
  'CURRENT',
  'ROW',
  'EXPLAIN',
  'ANALYZE',
  // Transaction keywords
  'BEGIN',
  'COMMIT',
  'ROLLBACK',
  'TRANSACTION',
  'ACTOR',
  'MESSAGE',
  // Tree manipulation keywords (ORDER, MOVE, COPY, TRANSLATE)
  'ABOVE',
  'BELOW',
  'TO',
  'MOVE',
  'COPY',
  'TREE',
  'BRANCH',
  'TRANSLATE',
  'FOR',
  'LOCALE',
  // Relation keywords (RELATE, UNRELATE)
  'RELATE',
  'UNRELATE',
  'WORKSPACE',
  'TYPE',
  'WEIGHT',
  // GRAPH_TABLE / PGQ keywords (SQL/PGQ standard)
  'GRAPH_TABLE',
  'MATCH',
  'COLUMNS',
  'NULLS',
  'FIRST',
  'LAST',
  'NULLIF',
  // Branch management keywords
  'SHOW',
  'BRANCHES',
  'DESCRIBE',
  'DIVERGENCE',
  'CHECKOUT',
  'USE',
  'HEAD',
  'UNSET',
  'AT',
  'REVISION',
  'PROTECTED',
  'UPSTREAM',
  'HISTORY',
  'LOCAL',
  'APP',
]

export const monarchTokenizer: languages.IMonarchLanguage = {
  defaultToken: 'text',
  ignoreCase: true,
  tokenPostfix: '.raisinsql',

  // Keyword groups for different highlighting
  statements: statementKeywords,
  schemaObjects: schemaObjectKeywords,
  clauses: clauseKeywords,
  propertyTypes: propertyTypeKeywords,
  modifiers: modifierKeywords,
  flags: flagKeywords,
  operators: operatorKeywords,
  functions: allFunctions,
  standardSql: standardSqlKeywords,
  cypherKeywords: cypherKeywords,
  cypherFunctions: cypherFunctions,

  // Brackets
  brackets: [
    { open: '[', close: ']', token: 'delimiter.square' },
    { open: '(', close: ')', token: 'delimiter.parenthesis' },
    { open: '{', close: '}', token: 'delimiter.curly' },
  ],

  tokenizer: {
    root: [
      // Whitespace
      { include: '@whitespace' },

      // Comments
      { include: '@comment' },

      // Numbers
      [/\d+(\.\d+)?/, 'number'],

      // CYPHER function with string argument - detect and enter Cypher highlighting mode
      // Match CYPHER followed by ( and whitespace, then transition based on quote type
      [/\bCYPHER\s*\(\s*'/, { token: 'function.cypher', next: '@cypherStringSingle' }],
      [/\bCYPHER\s*\(\s*"/, { token: 'function.cypher', next: '@cypherStringDouble' }],

      // Reference strings - detect 'workspace:/path' pattern after REFERENCES(
      // Format: workspace (no slash) : path (must start with slash)
      [/REFERENCES\s*\(\s*'/, { token: 'function', next: '@referenceString' }],

      // Strings - single quoted
      [/'([^'\\]|\\.)*$/, 'string.invalid'], // non-terminated
      [/'/, { token: 'string.quote', bracket: '@open', next: '@string' }],

      // Strings - double quoted (identifiers)
      [/"([^"\\]|\\.)*$/, 'string.invalid'], // non-terminated
      [/"/, { token: 'string.quote', bracket: '@open', next: '@stringDouble' }],

      // Namespace:Name patterns (like 'cms:Article')
      [
        /[a-zA-Z_][\w]*:[a-zA-Z_][\w]*/,
        'type.identifier', // namespace:type notation
      ],

      // Labels in GRAPH_TABLE patterns: :Label after ( or [ or |
      // This matches :Article, :User etc in (n:Article) or -[r:FOLLOWS]->
      [/:([A-Za-z_][A-Za-z0-9_]*)/, 'type.label.cypher'],

      // DDL Statement keywords (CREATE, ALTER, DROP)
      [
        /\b(CREATE|ALTER|DROP)\b/i,
        { cases: { '@statements': 'keyword.statement' } },
      ],

      // Schema object keywords (NODETYPE, ARCHETYPE, ELEMENTTYPE, BRANCH)
      [
        /\b(NODETYPE|ARCHETYPE|ELEMENTTYPE|BRANCH)\b/i,
        { cases: { '@schemaObjects': 'keyword.schemaObject' } },
      ],

      // Property types
      [
        /\b(String|Number|Boolean|Timestamp|Resource|Reference|Object|Array|Any|JSON|LocalizedString|RichText|Blocks|Relation|Vector)\b/,
        { cases: { '@propertyTypes': 'type.propertyType' } },
      ],

      // Modifiers (including branch merge strategies)
      [
        /\b(REQUIRED|FULLTEXT|INDEXED|UNIQUE|DEFAULT|CONSTRAINTS|DIMENSION|FAST_FORWARD|THREE_WAY|PROTECTED)\b/i,
        { cases: { '@modifiers': 'keyword.modifier' } },
      ],

      // Flags
      [
        /\b(CASCADE|ORDERABLE|ABSTRACT|SORTABLE|FILTERABLE|IF_NOT_EXISTS|IF_EXISTS)\b/i,
        { cases: { '@flags': 'keyword.flag' } },
      ],

      // Clause keywords (including branch-related clauses)
      [
        /\b(EXTENDS|PROPERTIES|PROPERTY|ADD|MODIFY|RENAME|TO|BASE_NODE_TYPE|TITLE|FIELDS|FIELD|UPSTREAM|INTO|USING|MESSAGE|REVISION|DESCRIPTION|HISTORY)\b/i,
        { cases: { '@clauses': 'keyword.clause' } },
      ],

      // Functions - special highlighting (including REFERENCES for reference index queries)
      [
        /\b(TO_JSON|TO_JSONB|JSONB_PATH_QUERY|JSONB_PATH_EXISTS|JSON_EXTRACT|CHILDREN|PARENT|ANCESTORS|DESCENDANTS|PATH|CHILD_OF|DESCENDANT_OF|REFERENCES|VECTOR_DISTANCE|VECTOR_SEARCH|COUNT|SUM|AVG|MIN|MAX|ARRAY_AGG|STRING_AGG|FIRST|LAST|ROW_NUMBER|RANK|DENSE_RANK|NTILE|LEAD|LAG)\b/i,
        { cases: { '@functions': 'function' } },
      ],

      // Standard SQL keywords (including branch management keywords)
      [
        /\b(SELECT|FROM|WHERE|AND|OR|NOT|IN|EXISTS|BETWEEN|LIKE|IS|NULL|TRUE|FALSE|AS|JOIN|LEFT|RIGHT|INNER|OUTER|CROSS|ON|GROUP|BY|HAVING|ORDER|ASC|DESC|LIMIT|OFFSET|UNION|ALL|DISTINCT|CASE|WHEN|THEN|ELSE|END|INSERT|UPSERT|UPDATE|DELETE|INTO|VALUES|SET|WITH|OVER|PARTITION|ROWS|RANGE|UNBOUNDED|PRECEDING|FOLLOWING|CURRENT|ROW|EXPLAIN|ANALYZE|MOVE|COPY|TREE|TO|ABOVE|BELOW|BRANCH|TRANSLATE|FOR|LOCALE|RELATE|UNRELATE|WORKSPACE|TYPE|WEIGHT|GRAPH_TABLE|MATCH|COLUMNS|NULLS|FIRST|LAST|NULLIF|MERGE|USE|CHECKOUT|SHOW|DESCRIBE|DIVERGENCE|HEAD|UNSET|AT|IF|BRANCHES|REVISION|PROTECTED|UPSTREAM|HISTORY|LOCAL|APP)\b/i,
        { cases: { '@standardSql': 'keyword' } },
      ],

      // Backtick-quoted identifiers (for namespace:type in GRAPH_TABLE labels)
      [/`[^`]+`/, 'type.identifier'],

      // Arrow operators for GRAPH_TABLE patterns (must come before general operators)
      [/->|<-/, 'operator.arrow.cypher'],

      // Operators
      [/[<>=!]+/, 'operator'],
      [/[+\-*/%]/, 'operator'],
      [/::/, 'operator.cast'], // PostgreSQL cast operator

      // Identifiers
      [/[a-zA-Z_][\w]*/, 'identifier'],

      // Delimiters
      [/[;,.]/, 'delimiter'],
      [/[()]/, '@brackets'],
      [/[[\]]/, '@brackets'],
      [/[{}]/, '@brackets'],
    ],

    whitespace: [[/\s+/, 'white']],

    comment: [
      [/--.*$/, 'comment'],
      [/\/\*/, 'comment', '@commentBlock'],
    ],

    commentBlock: [
      [/[^/*]+/, 'comment'],
      [/\*\//, 'comment', '@pop'],
      [/[/*]/, 'comment'],
    ],

    string: [
      [/[^'\\]+/, 'string'],
      [/\\./, 'string.escape'],
      [/'/, { token: 'string.quote', bracket: '@close', next: '@pop' }],
    ],

    stringDouble: [
      [/[^"\\]+/, 'string'],
      [/\\./, 'string.escape'],
      [/"/, { token: 'string.quote', bracket: '@close', next: '@pop' }],
    ],

    // Cypher string (single-quoted) - syntax highlighting for embedded Cypher queries
    cypherStringSingle: [
      // End of Cypher string - closing quote followed by closing paren
      [/'\s*\)/, { token: 'function.cypher', next: '@pop' }],
      // Escape sequences
      [/\\./, 'string.escape.cypher'],
      // Cypher keywords (case-insensitive)
      [
        /\b(MATCH|OPTIONAL|WHERE|RETURN|WITH|UNWIND|ORDER|BY|SKIP|LIMIT|CREATE|MERGE|DELETE|DETACH|SET|REMOVE|CALL|YIELD|UNION|ALL|AS|DISTINCT|CASE|WHEN|THEN|ELSE|END|AND|OR|XOR|NOT|IN|STARTS|ENDS|CONTAINS|IS|NULL|TRUE|FALSE|EXISTS|NONE|ANY|SINGLE|FOREACH|ON|ASC|DESC|ASCENDING|DESCENDING)\b/i,
        'keyword.cypher',
      ],
      // Cypher functions
      [
        /\b(id|type|labels|keys|properties|nodes|relationships|length|size|head|last|tail|range|reverse|toString|toInteger|toFloat|toBoolean|trim|ltrim|rtrim|replace|substring|left|right|split|toLower|toUpper|startNode|endNode|coalesce|timestamp|date|datetime|time|duration|point|distance|abs|ceil|floor|round|sign|rand|log|log10|exp|sqrt|sin|cos|tan|asin|acos|atan|atan2|degrees|radians|pi|e|shortestPath|allShortestPaths|count|sum|avg|min|max|collect)\b/i,
        'function.cypher',
      ],
      // Relationship patterns: ->, <-, -
      [/->|<-|-/, 'operator.arrow.cypher'],
      // Labels and relationship types :Label or :TYPE
      [/:([A-Za-z_][A-Za-z0-9_]*)/, 'type.label.cypher'],
      // Property access .property
      [/\.([A-Za-z_][A-Za-z0-9_]*)/, 'variable.property.cypher'],
      // Strings inside Cypher (double-quoted)
      [/"[^"]*"/, 'string.inner.cypher'],
      // Numbers
      [/\d+(\.\d+)?/, 'number.cypher'],
      // Operators
      [/[=<>!]+/, 'operator.cypher'],
      [/[+\-*/%]/, 'operator.cypher'],
      // Brackets and delimiters
      [/[()]/, 'delimiter.parenthesis.cypher'],
      [/[[\]]/, 'delimiter.bracket.cypher'],
      [/[{}]/, 'delimiter.curly.cypher'],
      // Variables/identifiers
      [/[a-zA-Z_][a-zA-Z0-9_]*/, 'variable.cypher'],
      // Whitespace
      [/\s+/, 'string.cypher'],
      // Any other character in the string
      [/./, 'string.cypher'],
    ],

    // Cypher string (double-quoted) - syntax highlighting for embedded Cypher queries
    cypherStringDouble: [
      // End of Cypher string - closing quote followed by closing paren
      [/"\s*\)/, { token: 'function.cypher', next: '@pop' }],
      // Escape sequences
      [/\\./, 'string.escape.cypher'],
      // Cypher keywords (case-insensitive)
      [
        /\b(MATCH|OPTIONAL|WHERE|RETURN|WITH|UNWIND|ORDER|BY|SKIP|LIMIT|CREATE|MERGE|DELETE|DETACH|SET|REMOVE|CALL|YIELD|UNION|ALL|AS|DISTINCT|CASE|WHEN|THEN|ELSE|END|AND|OR|XOR|NOT|IN|STARTS|ENDS|CONTAINS|IS|NULL|TRUE|FALSE|EXISTS|NONE|ANY|SINGLE|FOREACH|ON|ASC|DESC|ASCENDING|DESCENDING)\b/i,
        'keyword.cypher',
      ],
      // Cypher functions
      [
        /\b(id|type|labels|keys|properties|nodes|relationships|length|size|head|last|tail|range|reverse|toString|toInteger|toFloat|toBoolean|trim|ltrim|rtrim|replace|substring|left|right|split|toLower|toUpper|startNode|endNode|coalesce|timestamp|date|datetime|time|duration|point|distance|abs|ceil|floor|round|sign|rand|log|log10|exp|sqrt|sin|cos|tan|asin|acos|atan|atan2|degrees|radians|pi|e|shortestPath|allShortestPaths|count|sum|avg|min|max|collect)\b/i,
        'function.cypher',
      ],
      // Relationship patterns: ->, <-, -
      [/->|<-|-/, 'operator.arrow.cypher'],
      // Labels and relationship types :Label or :TYPE
      [/:([A-Za-z_][A-Za-z0-9_]*)/, 'type.label.cypher'],
      // Property access .property
      [/\.([A-Za-z_][A-Za-z0-9_]*)/, 'variable.property.cypher'],
      // Strings inside Cypher (single-quoted)
      [/'[^']*'/, 'string.inner.cypher'],
      // Numbers
      [/\d+(\.\d+)?/, 'number.cypher'],
      // Operators
      [/[=<>!]+/, 'operator.cypher'],
      [/[+\-*/%]/, 'operator.cypher'],
      // Brackets and delimiters
      [/[()]/, 'delimiter.parenthesis.cypher'],
      [/[[\]]/, 'delimiter.bracket.cypher'],
      [/[{}]/, 'delimiter.curly.cypher'],
      // Variables/identifiers
      [/[a-zA-Z_][a-zA-Z0-9_]*/, 'variable.cypher'],
      // Whitespace
      [/\s+/, 'string.cypher'],
      // Any other character in the string
      [/./, 'string.cypher'],
    ],

    // Reference string highlighting for REFERENCES('workspace:/path') format
    // Highlights workspace name, colon separator, and path differently
    referenceString: [
      // End of reference string - closing quote followed by closing paren
      [/'\s*\)/, { token: 'string.quote', next: '@pop' }],
      // Just closing quote (might have more text before paren)
      [/'/, { token: 'string.quote', next: '@pop' }],
      // Escape sequences
      [/\\./, 'string.escape'],
      // Workspace:path pattern - workspace (word chars, no slash), colon, path (starts with /)
      // Workspace name (before the colon) - highlighted as identifier/variable
      [/([a-zA-Z_][a-zA-Z0-9_-]*)(:)(\/[^\s'\\]*)/, ['variable.reference.workspace', 'operator.reference.separator', 'string.reference.path']],
      // Fallback: any other content in the string
      [/[^'\\]+/, 'string'],
    ],
  },
}
