import { useState, useEffect, useRef, useCallback } from 'react'
import { Plus, Trash2, AlertCircle, GripVertical, Network, X } from 'lucide-react'
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd'
import { clsx } from 'clsx'
import { RelEditor } from '../monaco/rel'

export interface ConditionBuilderProps {
  condition: string
  onChange: (newCondition: string) => void
  /** Field prefix for new rules. Default: 'input.' for workflows, use 'resource.' for permissions */
  fieldPrefix?: string
  /** Placeholder text for the text editor */
  placeholder?: string
}

type Operator = '==' | '!=' | '>' | '<' | '>=' | '<=' | 'contains' | 'startsWith' | 'endsWith'

interface Rule {
  id: string
  field: string
  operator: Operator
  value: string
}

interface RelatesRule {
  id: string
  type: 'relates'
  source: string           // e.g., "node.created_by"
  target: string           // e.g., "auth.local_user_id"
  relationTypes: string[]  // e.g., ["FRIENDS_WITH"]
  minDepth: number         // default: 1
  maxDepth: number         // default: 1
  direction: 'any' | 'outgoing' | 'incoming'
}

type RuleItem = Rule | RelatesRule | RuleGroup

interface RuleGroup {
  id: string
  combinator: '&&' | '||'
  rules: RuleItem[]
}

const OPERATORS: { label: string; value: Operator }[] = [
  { label: 'Equals', value: '==' },
  { label: 'Not Equals', value: '!=' },
  { label: 'Greater Than', value: '>' },
  { label: 'Less Than', value: '<' },
  { label: 'Greater/Equal', value: '>=' },
  { label: 'Less/Equal', value: '<=' },
  { label: 'Contains', value: 'contains' },
  { label: 'Starts With', value: 'startsWith' },
  { label: 'Ends With', value: 'endsWith' },
]

const generateId = () => Math.random().toString(36).substr(2, 9)

// Stop keyboard events from propagating to parent handlers (e.g., flow-designer delete key)
const stopKeyboardPropagation = (e: React.KeyboardEvent) => {
  e.stopPropagation()
}

// =============================================================================
// WASM Module Initialization
// =============================================================================

let wasmModule: typeof import('raisin-rel-wasm') | null = null
let wasmInitPromise: Promise<typeof import('raisin-rel-wasm')> | null = null

async function initWasm(): Promise<typeof import('raisin-rel-wasm')> {
  if (wasmModule) return wasmModule
  if (!wasmInitPromise) {
    wasmInitPromise = (async () => {
      const wasm = await import('raisin-rel-wasm')
      await wasm.default()
      wasmModule = wasm
      return wasm
    })()
  }
  return wasmInitPromise
}

// =============================================================================
// REL AST types from WASM parser (method-chaining syntax)
// =============================================================================

interface RelAst {
  BinaryOp?: {
    left: RelAst
    op: string
    right: RelAst
  }
  UnaryOp?: {
    op: string
    expr: RelAst
  }
  MethodCall?: {
    object: RelAst
    method: string
    args: RelAst[]
  }
  PropertyAccess?: {
    object: RelAst
    property: string
  }
  IndexAccess?: {
    object: RelAst
    index: RelAst
  }
  Variable?: string
  Literal?: RelLiteral
  Grouped?: RelAst
  Relates?: {
    source: RelAst
    target: RelAst
    relation_types: string[]
    min_depth: number
    max_depth: number
    direction: 'Any' | 'Outgoing' | 'Incoming'
  }
}

type RelLiteral =
  | { Null: null }
  | { Boolean: boolean }
  | { Integer: number }
  | { Float: number }
  | { String: string }
  | { Array: RelLiteral[] }
  | { Object: [string, RelLiteral][] }

// =============================================================================
// AST → RuleGroup conversion (for Text → Visual)
// =============================================================================

function astToRuleGroup(ast: RelAst): RuleGroup | null {
  // Handle grouped expressions by unwrapping
  if (ast.Grouped) {
    return astToRuleGroup(ast.Grouped)
  }

  // Handle AND/OR at top level
  if (ast.BinaryOp) {
    const { op, left, right } = ast.BinaryOp

    if (op === 'And' || op === 'Or') {
      const combinator = op === 'And' ? '&&' : '||'

      // Collect all rules with the same combinator
      const rules: RuleItem[] = []

      const collectRules = (node: RelAst, targetCombinator: string) => {
        if (node.Grouped) {
          // Grouped expressions become nested groups
          const subGroup = astToRuleGroup(node.Grouped)
          if (subGroup) {
            rules.push(subGroup)
          }
          return
        }

        if (node.BinaryOp && (node.BinaryOp.op === 'And' || node.BinaryOp.op === 'Or')) {
          if (node.BinaryOp.op === targetCombinator) {
            // Same combinator - flatten
            collectRules(node.BinaryOp.left, targetCombinator)
            collectRules(node.BinaryOp.right, targetCombinator)
          } else {
            // Different combinator - create nested group
            const subGroup = astToRuleGroup(node)
            if (subGroup) {
              rules.push(subGroup)
            }
          }
        } else {
          // It's a comparison, function call, or RELATES - convert to rule
          const rule = astToRule(node)
          if (rule) {
            rules.push(rule as RuleItem)
          }
        }
      }

      collectRules(left, op)
      collectRules(right, op)

      if (rules.length > 0) {
        return {
          id: generateId(),
          combinator,
          rules,
        }
      }
    }

    // Single comparison expression
    const rule = astToRule(ast)
    if (rule) {
      return {
        id: 'root',
        combinator: '&&',
        rules: [rule],
      }
    }
  }

  // Handle method calls (contains, startsWith, endsWith)
  if (ast.MethodCall) {
    const rule = astToRule(ast)
    if (rule) {
      return {
        id: 'root',
        combinator: '&&',
        rules: [rule],
      }
    }
  }

  return null
}

function astToRule(ast: RelAst): Rule | RelatesRule | null {
  // Handle RELATES expression
  if (ast.Relates) {
    const { source, target, relation_types, min_depth, max_depth, direction } = ast.Relates
    const sourceField = astToFieldPath(source)
    const targetField = astToFieldPath(target)

    if (sourceField !== null && targetField !== null) {
      const directionMap: Record<string, 'any' | 'outgoing' | 'incoming'> = {
        'Any': 'any',
        'Outgoing': 'outgoing',
        'Incoming': 'incoming',
      }

      return {
        id: generateId(),
        type: 'relates',
        source: sourceField,
        target: targetField,
        relationTypes: relation_types,
        minDepth: min_depth,
        maxDepth: max_depth,
        direction: directionMap[direction] || 'any',
      }
    }
  }

  // Handle comparison operators
  if (ast.BinaryOp) {
    const { op, left, right } = ast.BinaryOp

    const opMap: Record<string, Operator> = {
      'Eq': '==',
      'Neq': '!=',
      'Lt': '<',
      'Gt': '>',
      'Lte': '<=',
      'Gte': '>=',
    }

    if (opMap[op]) {
      const field = astToFieldPath(left)
      const value = astToValue(right)

      if (field !== null && value !== null) {
        return {
          id: generateId(),
          field,
          operator: opMap[op],
          value,
        }
      }
    }
  }

  // Handle method calls (method-chaining syntax: input.field.contains('value'))
  if (ast.MethodCall) {
    const { object, method, args } = ast.MethodCall
    const methodLower = method.toLowerCase()

    if (['contains', 'startswith', 'endswith'].includes(methodLower) && args.length === 1) {
      const field = astToFieldPath(object)
      const value = astToValue(args[0])

      const opMap: Record<string, Operator> = {
        'contains': 'contains',
        'startswith': 'startsWith',
        'endswith': 'endsWith',
      }

      if (field !== null && value !== null) {
        return {
          id: generateId(),
          field,
          operator: opMap[methodLower],
          value,
        }
      }
    }
  }

  return null
}

function astToFieldPath(ast: RelAst): string | null {
  if (ast.Variable) {
    return ast.Variable
  }

  if (ast.PropertyAccess) {
    const base = astToFieldPath(ast.PropertyAccess.object)
    if (base) {
      return `${base}.${ast.PropertyAccess.property}`
    }
  }

  if (ast.IndexAccess) {
    const base = astToFieldPath(ast.IndexAccess.object)
    const index = astToValue(ast.IndexAccess.index)
    if (base && index !== null) {
      return `${base}[${index}]`
    }
  }

  return null
}

function astToValue(ast: RelAst): string | null {
  if (ast.Literal) {
    const lit = ast.Literal
    if ('Null' in lit) return 'null'
    if ('Boolean' in lit) return String(lit.Boolean)
    if ('Integer' in lit) return String(lit.Integer)
    if ('Float' in lit) return String(lit.Float)
    if ('String' in lit) return lit.String
    // Arrays and objects are too complex for visual mode
    return null
  }

  // Also allow field paths as values
  const fieldPath = astToFieldPath(ast)
  if (fieldPath) {
    return fieldPath
  }

  return null
}

// =============================================================================
// RuleGroup → AST conversion (for Visual → Text via WASM stringify)
// =============================================================================

function ruleGroupToAst(group: RuleGroup): RelAst {
  if (group.rules.length === 0) {
    // Empty group = always true
    return { Literal: { Boolean: true } }
  }

  if (group.rules.length === 1) {
    const item = group.rules[0]
    if ('combinator' in item) {
      // Nested group - wrap in Grouped
      return { Grouped: ruleGroupToAst(item) }
    } else {
      return ruleToAst(item)
    }
  }

  // Build binary tree: ((a && b) && c)
  const op = group.combinator === '&&' ? 'And' : 'Or'

  const astItems = group.rules.map(item => {
    if ('combinator' in item) {
      // Nested group - wrap in Grouped
      return { Grouped: ruleGroupToAst(item) }
    } else {
      return ruleToAst(item)
    }
  })

  // Reduce to binary tree
  return astItems.reduce((left, right, index) => {
    if (index === 0) return left
    return {
      BinaryOp: {
        left,
        op,
        right,
      },
    }
  })
}

function isRelatesRule(rule: RuleItem): rule is RelatesRule {
  return 'type' in rule && rule.type === 'relates'
}

function isRegularRule(rule: RuleItem): rule is Rule {
  return 'field' in rule && 'operator' in rule && 'value' in rule
}

function ruleToAst(rule: RuleItem): RelAst {
  // Handle RelatesRule
  if (isRelatesRule(rule)) {
    const directionMap: Record<'any' | 'outgoing' | 'incoming', 'Any' | 'Outgoing' | 'Incoming'> = {
      'any': 'Any',
      'outgoing': 'Outgoing',
      'incoming': 'Incoming',
    }

    return {
      Relates: {
        source: fieldPathToAst(rule.source),
        target: fieldPathToAst(rule.target),
        relation_types: rule.relationTypes,
        min_depth: rule.minDepth,
        max_depth: rule.maxDepth,
        direction: directionMap[rule.direction],
      },
    }
  }

  // Handle regular Rule
  if (!isRegularRule(rule)) {
    // Should never happen, but return a safe default
    return { Literal: { Boolean: true } }
  }

  const field = fieldPathToAst(rule.field)
  const value = valueToAst(rule.value)

  // Method operators
  if (['contains', 'startsWith', 'endsWith'].includes(rule.operator)) {
    return {
      MethodCall: {
        object: field,
        method: rule.operator,
        args: [value],
      },
    }
  }

  // Comparison operators
  const opMap: Record<string, string> = {
    '==': 'Eq',
    '!=': 'Neq',
    '<': 'Lt',
    '>': 'Gt',
    '<=': 'Lte',
    '>=': 'Gte',
  }

  return {
    BinaryOp: {
      left: field,
      op: opMap[rule.operator],
      right: value,
    },
  }
}

function fieldPathToAst(path: string): RelAst {
  // Handle index access like input.array[0]
  const indexMatch = path.match(/^(.+)\[(\d+|'[^']+'|"[^"]+")\]$/)
  if (indexMatch) {
    const basePath = indexMatch[1]
    const indexStr = indexMatch[2]
    const base = fieldPathToAst(basePath)

    // Parse index - could be number or string
    let indexAst: RelAst
    if (/^\d+$/.test(indexStr)) {
      indexAst = { Literal: { Integer: parseInt(indexStr, 10) } }
    } else {
      // Strip quotes from string index
      const strVal = indexStr.slice(1, -1)
      indexAst = { Literal: { String: strVal } }
    }

    return {
      IndexAccess: {
        object: base,
        index: indexAst,
      },
    }
  }

  // Handle property access like input.foo.bar
  const parts = path.split('.')
  let ast: RelAst = { Variable: parts[0] }

  for (let i = 1; i < parts.length; i++) {
    ast = {
      PropertyAccess: {
        object: ast,
        property: parts[i],
      },
    }
  }

  return ast
}

function valueToAst(value: string): RelAst {
  if (value === 'null') return { Literal: { Null: null } }
  if (value === 'true') return { Literal: { Boolean: true } }
  if (value === 'false') return { Literal: { Boolean: false } }

  // Check if it's a number
  if (value !== '' && !isNaN(Number(value))) {
    if (value.includes('.')) {
      return { Literal: { Float: parseFloat(value) } }
    } else {
      return { Literal: { Integer: parseInt(value, 10) } }
    }
  }

  // Check if it's a field path reference (e.g., input.other.value)
  if (/^[\w.]+$/.test(value) && value.includes('.')) {
    return fieldPathToAst(value)
  }

  // String literal
  return { Literal: { String: value } }
}

// =============================================================================
// WASM-based parsing and stringifying
// =============================================================================

async function parseConditionWithWasm(condition: string): Promise<RuleGroup | null> {
  if (!condition || !condition.trim()) return null

  try {
    const wasm = await initWasm()
    const result = wasm.parse_expression(condition)

    if (result.success && result.ast) {
      return astToRuleGroup(result.ast)
    }
  } catch (e) {
    // WASM module error - fall through to return null
  }

  return null
}

async function stringifyWithWasm(group: RuleGroup): Promise<string> {
  if (group.rules.length === 0) return ''

  try {
    const wasm = await initWasm()
    const ast = ruleGroupToAst(group)
    const result = wasm.stringify_expression(ast)

    if (result.success && result.code) {
      return result.code
    }
  } catch {
    // WASM module error - fall through to return empty
  }

  return ''
}

async function validateWithWasm(expression: string): Promise<ValidationError[]> {
  if (!expression || !expression.trim()) return []

  try {
    const wasm = await initWasm()
    const result = wasm.validate_expression(expression)

    if (!result.valid && result.errors) {
      return result.errors.map((err: { line: number; column: number; end_line: number; end_column: number; message: string }) => ({
        line: err.line,
        column: err.column,
        endLine: err.end_line,
        endColumn: err.end_column,
        message: err.message,
      }))
    }
  } catch {
    // WASM module error - no validation errors to report
  }

  return []
}

// Validation error type matching RelEditor's expected format
interface ValidationError {
  line: number
  column: number
  endLine: number
  endColumn: number
  message: string
}

// =============================================================================
// Component
// =============================================================================

export function ConditionBuilder({
  condition,
  onChange,
  fieldPrefix = 'input.',
  placeholder,
}: ConditionBuilderProps) {
  const defaultPlaceholder = placeholder || `${fieldPrefix}value > 10 && ${fieldPrefix}status == 'active'`

  const createDefaultRule = useCallback((): Rule => ({
    id: generateId(),
    field: fieldPrefix,
    operator: '==',
    value: '',
  }), [fieldPrefix])

  const createDefaultGroup = useCallback((): RuleGroup => ({
    id: 'root',
    combinator: '&&',
    rules: [createDefaultRule()],
  }), [createDefaultRule])

  const [mode, setMode] = useState<'visual' | 'text'>('visual')
  const [rootGroup, setRootGroup] = useState<RuleGroup>(createDefaultGroup)
  const [parseError, setParseError] = useState<string | null>(null)
  const [validationErrors, setValidationErrors] = useState<ValidationError[]>([])
  const [isLoading, setIsLoading] = useState(false)
  const lastEmittedRef = useRef<string | null>(null)
  const isInitialMount = useRef(true)

  // Parse condition on mount and when condition changes externally
  useEffect(() => {
    if (!condition) {
      // Empty condition - default state
      setValidationErrors([])
      if (isInitialMount.current) {
        isInitialMount.current = false
      }
      return
    }

    // If this update matches what we just emitted, ignore it
    if (condition === lastEmittedRef.current) return

    // Validate for error markers
    validateWithWasm(condition).then(setValidationErrors)

    // Try to parse with WASM
    parseConditionWithWasm(condition).then(parsed => {
      if (parsed) {
        setRootGroup(parsed)
        setMode('visual')
        setParseError(null)
      } else {
        // Could not parse, switch to text mode
        setMode('text')
      }
    })

    isInitialMount.current = false
  }, [condition])

  const handleGroupChange = useCallback(async (newGroup: RuleGroup) => {
    setRootGroup(newGroup)
    const stringified = await stringifyWithWasm(newGroup)
    lastEmittedRef.current = stringified
    onChange(stringified)
  }, [onChange])

  const onDragEnd = (result: DropResult) => {
    const { source, destination } = result
    if (!destination) return

    // Deep clone to mutate
    const newRoot = JSON.parse(JSON.stringify(rootGroup)) as RuleGroup

    // Helper to find group by ID
    const findGroup = (group: RuleGroup, id: string): RuleGroup | null => {
      if (group.id === id) return group
      for (const rule of group.rules) {
        if ('combinator' in rule) {
          const found = findGroup(rule, id)
          if (found) return found
        }
      }
      return null
    }

    const sourceGroup = findGroup(newRoot, source.droppableId)
    const destGroup = findGroup(newRoot, destination.droppableId)

    if (!sourceGroup || !destGroup) return

    // Move item
    const [movedItem] = sourceGroup.rules.splice(source.index, 1)
    destGroup.rules.splice(destination.index, 0, movedItem)

    handleGroupChange(newRoot)
  }

  const switchToVisual = useCallback(async () => {
    if (!condition || !condition.trim()) {
      setRootGroup(createDefaultGroup())
      setMode('visual')
      setParseError(null)
      return
    }

    setIsLoading(true)
    setParseError(null)

    try {
      const parsed = await parseConditionWithWasm(condition)

      if (parsed) {
        setRootGroup(parsed)
        setMode('visual')
        setParseError(null)
      } else {
        setParseError('Cannot parse this expression visually. It may use unsupported features like nested groups or complex literals.')
      }
    } catch {
      setParseError('Cannot parse this expression visually. Try simplifying it.')
    } finally {
      setIsLoading(false)
    }
  }, [condition, createDefaultGroup])

  const handleTextChange = useCallback(async (newValue: string) => {
    lastEmittedRef.current = newValue
    onChange(newValue)

    // Validate and update error markers
    const errors = await validateWithWasm(newValue)
    setValidationErrors(errors)
  }, [onChange])

  if (mode === 'text') {
    return (
      <div className="space-y-2">
        <div className="flex justify-between items-center">
          <label className="text-xs text-gray-500">Expression (REL)</label>
          <div className="flex items-center gap-2">
            {parseError && (
              <span className="text-xs text-red-400 flex items-center gap-1">
                <AlertCircle className="w-3 h-3" />
                {parseError}
              </span>
            )}
            <button
              onClick={switchToVisual}
              disabled={isLoading}
              className="text-xs text-blue-400 hover:text-blue-300 disabled:opacity-50"
            >
              {isLoading ? 'Parsing...' : 'Switch to Visual'}
            </button>
          </div>
        </div>
        <div
          className="bg-black/30 border border-white/10 rounded-lg overflow-hidden"
          onKeyDown={stopKeyboardPropagation}
          onKeyUp={stopKeyboardPropagation}
        >
          <RelEditor
            value={condition}
            onChange={handleTextChange}
            height="100px"
            placeholder={defaultPlaceholder}
            errors={validationErrors}
          />
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-2">
      <div className="flex justify-between items-center mb-2">
        <span className="text-xs font-medium text-gray-400">Rules Engine</span>
        <button
          onClick={() => setMode('text')}
          className="text-xs text-blue-400 hover:text-blue-300"
        >
          Edit as Text
        </button>
      </div>
      <div className="bg-white/5 border border-white/10 rounded-lg p-3">
        <DragDropContext onDragEnd={onDragEnd}>
          <GroupEditor
            group={rootGroup}
            onChange={handleGroupChange}
            isRoot
            fieldPrefix={fieldPrefix}
          />
        </DragDropContext>
      </div>
    </div>
  )
}

function GroupEditor({
  group,
  onChange,
  isRoot = false,
  onDelete,
  dragHandleProps,
  fieldPrefix = 'input.',
}: {
  group: RuleGroup
  onChange: (g: RuleGroup) => void
  isRoot?: boolean
  onDelete?: () => void
  dragHandleProps?: any
  fieldPrefix?: string
}) {
  const updateCombinator = (combinator: '&&' | '||') => {
    onChange({ ...group, combinator })
  }

  const addRule = () => {
    onChange({
      ...group,
      rules: [...group.rules, { id: generateId(), field: fieldPrefix, operator: '==', value: '' }]
    })
  }

  const addGroup = () => {
    onChange({
      ...group,
      rules: [...group.rules, {
        id: generateId(),
        combinator: '&&',
        rules: [{ id: generateId(), field: fieldPrefix, operator: '==', value: '' }]
      }]
    })
  }

  const addRelatesRule = () => {
    onChange({
      ...group,
      rules: [...group.rules, {
        id: generateId(),
        type: 'relates' as const,
        source: 'node.created_by',
        target: 'auth.local_user_id',
        relationTypes: [],
        minDepth: 1,
        maxDepth: 1,
        direction: 'any' as const,
      }]
    })
  }

  const updateRule = (index: number, newRule: Rule | RelatesRule) => {
    const newRules = [...group.rules]
    newRules[index] = newRule
    onChange({ ...group, rules: newRules })
  }

  const updateSubGroup = (index: number, newGroup: RuleGroup) => {
    const newRules = [...group.rules]
    newRules[index] = newGroup
    onChange({ ...group, rules: newRules })
  }

  const removeItem = (index: number) => {
    const newRules = [...group.rules]
    newRules.splice(index, 1)
    onChange({ ...group, rules: newRules })
  }

  return (
    <div className={clsx("space-y-3", !isRoot && "pl-4 border-l-2 border-white/10 ml-1 mt-2")}>
      <div className="flex items-center gap-2">
        {!isRoot && (
          <div {...dragHandleProps} className="cursor-grab text-gray-600 hover:text-gray-400 mr-1">
            <GripVertical className="w-4 h-4" />
          </div>
        )}
        <div className="flex bg-black/40 rounded-md p-0.5 border border-white/10">
          <button
            onClick={() => updateCombinator('&&')}
            className={clsx(
              "px-2 py-0.5 text-xs rounded-sm transition-colors",
              group.combinator === '&&' ? "bg-blue-500/20 text-blue-400" : "text-gray-500 hover:text-gray-300"
            )}
          >
            AND
          </button>
          <button
            onClick={() => updateCombinator('||')}
            className={clsx(
              "px-2 py-0.5 text-xs rounded-sm transition-colors",
              group.combinator === '||' ? "bg-blue-500/20 text-blue-400" : "text-gray-500 hover:text-gray-300"
            )}
          >
            OR
          </button>
        </div>

        <div className="flex-1" />

        <button onClick={addRule} className="text-xs text-gray-400 hover:text-white flex items-center gap-1">
          <Plus className="w-3 h-3" /> Rule
        </button>
        <button onClick={addGroup} className="text-xs text-gray-400 hover:text-white flex items-center gap-1">
          <Plus className="w-3 h-3" /> Group
        </button>
        <button onClick={addRelatesRule} className="text-xs text-gray-400 hover:text-white flex items-center gap-1">
          <Network className="w-3 h-3" /> Relation Check
        </button>
        {!isRoot && onDelete && (
          <button onClick={onDelete} className="text-gray-500 hover:text-red-400 ml-2">
            <Trash2 className="w-3 h-3" />
          </button>
        )}
      </div>

      <Droppable droppableId={group.id}>
        {(provided) => (
          <div
            className="space-y-2"
            ref={provided.innerRef}
            {...provided.droppableProps}
          >
            {group.rules.map((item, index) => (
              <Draggable key={item.id} draggableId={item.id} index={index}>
                {(provided, snapshot) => (
                  <div
                    ref={provided.innerRef}
                    {...provided.draggableProps}
                    className={clsx(
                      snapshot.isDragging && "bg-gray-900 rounded-lg border border-white/20"
                    )}
                  >
                    {'combinator' in item ? (
                      <GroupEditor
                        group={item}
                        onChange={(g) => updateSubGroup(index, g)}
                        onDelete={() => removeItem(index)}
                        dragHandleProps={provided.dragHandleProps}
                        fieldPrefix={fieldPrefix}
                      />
                    ) : 'type' in item && item.type === 'relates' ? (
                      <RelatesEditor
                        rule={item}
                        onChange={(r) => updateRule(index, r)}
                        onDelete={() => removeItem(index)}
                        dragHandleProps={provided.dragHandleProps}
                      />
                    ) : (
                      <RuleEditor
                        rule={item as Rule}
                        onChange={(r) => updateRule(index, r)}
                        onDelete={() => removeItem(index)}
                        dragHandleProps={provided.dragHandleProps}
                      />
                    )}
                  </div>
                )}
              </Draggable>
            ))}
            {provided.placeholder}
            {group.rules.length === 0 && (
              <div className="text-xs text-gray-600 italic py-1">No rules</div>
            )}
          </div>
        )}
      </Droppable>
    </div>
  )
}

function RelatesEditor({ rule, onChange, onDelete, dragHandleProps }: { rule: RelatesRule, onChange: (r: RelatesRule) => void, onDelete: () => void, dragHandleProps?: any }) {
  const [tagInput, setTagInput] = useState('')

  const handleAddTag = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' && tagInput.trim()) {
      e.preventDefault()
      if (!rule.relationTypes.includes(tagInput.trim())) {
        onChange({ ...rule, relationTypes: [...rule.relationTypes, tagInput.trim()] })
      }
      setTagInput('')
    }
  }

  const handleRemoveTag = (tag: string) => {
    onChange({ ...rule, relationTypes: rule.relationTypes.filter(t => t !== tag) })
  }

  return (
    <div
      className="bg-blue-50/10 backdrop-blur-sm border border-blue-500/20 rounded-lg p-3 space-y-2"
      onKeyDown={stopKeyboardPropagation}
      onKeyUp={stopKeyboardPropagation}
    >
      <div className="flex items-center gap-2">
        <div {...dragHandleProps} className="cursor-grab text-gray-600 hover:text-gray-400">
          <GripVertical className="w-4 h-4" />
        </div>
        <Network className="w-4 h-4 text-cyan-400" />
        <span className="text-xs font-medium text-gray-400">Graph Relation Check</span>
        <div className="flex-1" />
        <button onClick={onDelete} className="text-gray-500 hover:text-red-400">
          <Trash2 className="w-3 h-3" />
        </button>
      </div>

      <div className="flex items-center gap-2 flex-wrap">
        <input
          type="text"
          value={rule.source}
          onChange={(e) => onChange({ ...rule, source: e.target.value })}
          className="flex-1 min-w-[120px] bg-black/20 border border-white/10 rounded text-xs text-white focus:border-cyan-500 focus:outline-none px-2 py-1"
          placeholder="node.created_by"
        />

        <span className="text-xs font-bold text-cyan-400">RELATES</span>

        <input
          type="text"
          value={rule.target}
          onChange={(e) => onChange({ ...rule, target: e.target.value })}
          className="flex-1 min-w-[120px] bg-black/20 border border-white/10 rounded text-xs text-white focus:border-cyan-500 focus:outline-none px-2 py-1"
          placeholder="auth.local_user_id"
        />
      </div>

      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-xs font-medium text-blue-400">VIA</span>

        <div className="flex flex-wrap gap-1 items-center flex-1">
          {rule.relationTypes.map(tag => (
            <span
              key={tag}
              className="inline-flex items-center gap-1 bg-cyan-500/20 text-cyan-300 text-xs px-2 py-0.5 rounded-full border border-cyan-500/30"
            >
              {tag}
              <button
                onClick={() => handleRemoveTag(tag)}
                className="hover:text-cyan-100"
              >
                <X className="w-3 h-3" />
              </button>
            </span>
          ))}
          <input
            type="text"
            value={tagInput}
            onChange={(e) => setTagInput(e.target.value)}
            onKeyDown={handleAddTag}
            className="flex-1 min-w-[100px] bg-black/20 border border-white/10 rounded text-xs text-white focus:border-cyan-500 focus:outline-none px-2 py-1"
            placeholder="FRIENDS_WITH (press Enter)"
          />
        </div>
      </div>

      <div className="flex items-center gap-3 flex-wrap">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-blue-400">DEPTH</span>
          <input
            type="number"
            min="1"
            value={rule.minDepth}
            onChange={(e) => onChange({ ...rule, minDepth: Math.max(1, parseInt(e.target.value) || 1) })}
            className="w-16 bg-black/20 border border-white/10 rounded text-xs text-white focus:border-cyan-500 focus:outline-none px-2 py-1"
          />
          <span className="text-xs text-gray-500">to</span>
          <input
            type="number"
            min="1"
            value={rule.maxDepth}
            onChange={(e) => onChange({ ...rule, maxDepth: Math.max(1, parseInt(e.target.value) || 1) })}
            className="w-16 bg-black/20 border border-white/10 rounded text-xs text-white focus:border-cyan-500 focus:outline-none px-2 py-1"
          />
        </div>

        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-blue-400">DIRECTION</span>
          <select
            value={rule.direction}
            onChange={(e) => onChange({ ...rule, direction: e.target.value as 'any' | 'outgoing' | 'incoming' })}
            className="bg-black/40 border border-white/10 rounded text-xs text-gray-300 focus:outline-none px-2 py-1"
          >
            <option value="any">Any</option>
            <option value="outgoing">Outgoing →</option>
            <option value="incoming">← Incoming</option>
          </select>
        </div>
      </div>
    </div>
  )
}

function RuleEditor({ rule, onChange, onDelete, dragHandleProps }: { rule: Rule, onChange: (r: Rule) => void, onDelete: () => void, dragHandleProps?: any }) {
  return (
    <div
      className="flex items-center gap-2 bg-black/20 p-2 rounded border border-white/5"
      onKeyDown={stopKeyboardPropagation}
      onKeyUp={stopKeyboardPropagation}
    >
      <div {...dragHandleProps} className="cursor-grab text-gray-600 hover:text-gray-400">
        <GripVertical className="w-4 h-4" />
      </div>
      <input
        type="text"
        value={rule.field}
        onChange={(e) => onChange({ ...rule, field: e.target.value })}
        className="flex-1 min-w-[80px] bg-transparent border-b border-white/10 text-xs text-white focus:border-blue-500 focus:outline-none px-1 py-0.5"
        placeholder="field"
      />

      <select
        value={rule.operator}
        onChange={(e) => onChange({ ...rule, operator: e.target.value as Operator })}
        className="bg-black/40 border border-white/10 rounded text-xs text-gray-300 focus:outline-none px-1 py-0.5"
      >
        {OPERATORS.map(op => (
          <option key={op.value} value={op.value}>{op.label}</option>
        ))}
      </select>

      <input
        type="text"
        value={rule.value}
        onChange={(e) => onChange({ ...rule, value: e.target.value })}
        className="flex-1 min-w-[80px] bg-transparent border-b border-white/10 text-xs text-white focus:border-blue-500 focus:outline-none px-1 py-0.5"
        placeholder="value"
      />

      <button onClick={onDelete} className="text-gray-500 hover:text-red-400">
        <Trash2 className="w-3 h-3" />
      </button>
    </div>
  )
}
