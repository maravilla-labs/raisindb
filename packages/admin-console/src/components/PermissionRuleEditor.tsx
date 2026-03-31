import { Plus, Trash2, Info } from 'lucide-react'

// Permission rule types matching the raisin-messaging schema
export type PermissionRuleType =
  | 'always'
  | 'never'
  | 'relationship'
  | 'same_group'
  | 'same_role'
  | 'sender_has_role'
  | 'recipient_has_role'

export interface PermissionRule {
  type: PermissionRuleType
  relation?: string // for 'relationship' type
  group_type?: string // for 'same_group' type
  role?: string // for 'same_role', 'sender_has_role', 'recipient_has_role' types
}

export interface PermissionRuleSet {
  mode: 'any_of' | 'all_of'
  rules: PermissionRule[]
}

interface PermissionRuleEditorProps {
  title: string
  description: string
  value: PermissionRuleSet
  onChange: (value: PermissionRuleSet) => void
  relationTypes?: string[] // Available relation types for dropdown
  expanded: boolean
  onToggle: () => void
  icon: React.ComponentType<{ className?: string }>
  iconColor: string // e.g., 'blue', 'green', 'purple'
}

const RULE_TYPES: { value: PermissionRuleType; label: string; description: string }[] = [
  { value: 'always', label: 'Always Allow', description: 'No restrictions' },
  { value: 'never', label: 'Never Allow', description: 'Block all messages' },
  { value: 'relationship', label: 'Relationship', description: 'Require specific relationship' },
  { value: 'same_group', label: 'Same Group', description: 'Must be in same group' },
  { value: 'same_role', label: 'Same Role', description: 'Both users have role' },
  { value: 'sender_has_role', label: 'Sender Has Role', description: 'Sender must have role' },
  { value: 'recipient_has_role', label: 'Recipient Has Role', description: 'Recipient must have role' },
]

const DEFAULT_RELATION_TYPES = [
  'FRIENDS_WITH',
  'FOLLOWS',
  'PARENT_OF',
  'GUARDIAN_OF',
  'MANAGER_OF',
  'SPOUSE_OF',
  'SIBLING_OF',
]

export default function PermissionRuleEditor({
  title,
  description,
  value,
  onChange,
  relationTypes = DEFAULT_RELATION_TYPES,
  expanded,
  onToggle,
  icon: Icon,
  iconColor,
}: PermissionRuleEditorProps) {
  const updateMode = (mode: 'any_of' | 'all_of') => {
    onChange({ ...value, mode })
  }

  const addRule = () => {
    onChange({
      ...value,
      rules: [...value.rules, { type: 'always' }],
    })
  }

  const updateRule = (index: number, rule: PermissionRule) => {
    const newRules = [...value.rules]
    newRules[index] = rule
    onChange({ ...value, rules: newRules })
  }

  const removeRule = (index: number) => {
    onChange({
      ...value,
      rules: value.rules.filter((_, i) => i !== index),
    })
  }

  const getColorClasses = (color: string) => {
    const colors: Record<string, { bg: string; border: string; text: string }> = {
      blue: { bg: 'bg-blue-500/20', border: 'border-blue-500/30', text: 'text-blue-400' },
      green: { bg: 'bg-green-500/20', border: 'border-green-500/30', text: 'text-green-400' },
      purple: { bg: 'bg-purple-500/20', border: 'border-purple-500/30', text: 'text-purple-400' },
      amber: { bg: 'bg-amber-500/20', border: 'border-amber-500/30', text: 'text-amber-400' },
      cyan: { bg: 'bg-cyan-500/20', border: 'border-cyan-500/30', text: 'text-cyan-400' },
    }
    return colors[color] || colors.blue
  }

  const colors = getColorClasses(iconColor)

  return (
    <div className="bg-white/5 backdrop-blur-sm rounded-xl border border-white/10 overflow-hidden">
      <button
        onClick={onToggle}
        className="w-full flex items-center justify-between p-4 hover:bg-white/5 transition-colors"
      >
        <div className="flex items-center gap-3">
          <div className={`p-2 rounded-lg ${colors.bg} border ${colors.border}`}>
            <Icon className={`w-5 h-5 ${colors.text}`} />
          </div>
          <div className="text-left">
            <h3 className="text-white font-medium">{title}</h3>
            <p className="text-sm text-gray-400">{description}</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-xs text-gray-500 bg-white/5 px-2 py-1 rounded">
            {value.rules.length} rule{value.rules.length !== 1 ? 's' : ''}
          </span>
          <svg
            className={`w-5 h-5 text-gray-400 transition-transform ${expanded ? 'rotate-180' : ''}`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </div>
      </button>

      {expanded && (
        <div className="p-4 pt-0 space-y-4">
          {/* Mode selector */}
          <div className="flex items-center gap-4 p-3 bg-white/5 rounded-lg">
            <span className="text-sm text-gray-300">Match:</span>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="radio"
                name={`${title}-mode`}
                checked={value.mode === 'any_of'}
                onChange={() => updateMode('any_of')}
                className="w-4 h-4 text-purple-500 border-white/20 bg-white/5 focus:ring-purple-400"
              />
              <span className="text-sm text-white">Any rule (OR)</span>
            </label>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="radio"
                name={`${title}-mode`}
                checked={value.mode === 'all_of'}
                onChange={() => updateMode('all_of')}
                className="w-4 h-4 text-purple-500 border-white/20 bg-white/5 focus:ring-purple-400"
              />
              <span className="text-sm text-white">All rules (AND)</span>
            </label>
          </div>

          {/* Rules list */}
          <div className="space-y-3">
            {value.rules.map((rule, index) => (
              <RuleRow
                key={index}
                rule={rule}
                relationTypes={relationTypes}
                onChange={(r) => updateRule(index, r)}
                onRemove={() => removeRule(index)}
              />
            ))}

            {value.rules.length === 0 && (
              <div className="flex items-start gap-2 p-3 bg-amber-500/10 border border-amber-500/30 rounded-lg text-sm">
                <Info className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
                <p className="text-amber-200/80">
                  No rules configured. Add at least one rule to define permissions.
                </p>
              </div>
            )}
          </div>

          {/* Add rule button */}
          <button
            onClick={addRule}
            className="flex items-center gap-2 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg text-gray-300 hover:text-white transition-colors"
          >
            <Plus className="w-4 h-4" />
            Add Rule
          </button>
        </div>
      )}
    </div>
  )
}

interface RuleRowProps {
  rule: PermissionRule
  relationTypes: string[]
  onChange: (rule: PermissionRule) => void
  onRemove: () => void
}

function RuleRow({ rule, relationTypes, onChange, onRemove }: RuleRowProps) {
  const needsRelation = rule.type === 'relationship'
  const needsGroupType = rule.type === 'same_group'
  const needsRole = ['same_role', 'sender_has_role', 'recipient_has_role'].includes(rule.type)

  return (
    <div className="flex items-center gap-3 p-3 bg-white/5 rounded-lg border border-white/10">
      {/* Rule type dropdown */}
      <select
        value={rule.type}
        onChange={(e) => {
          const newType = e.target.value as PermissionRuleType
          onChange({ type: newType })
        }}
        className="px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all min-w-[160px]"
      >
        {RULE_TYPES.map((rt) => (
          <option key={rt.value} value={rt.value} className="bg-gray-900">
            {rt.label}
          </option>
        ))}
      </select>

      {/* Relation type dropdown (for 'relationship' type) */}
      {needsRelation && (
        <select
          value={rule.relation || ''}
          onChange={(e) => onChange({ ...rule, relation: e.target.value })}
          className="px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all flex-1"
        >
          <option value="" className="bg-gray-900">
            Select relation...
          </option>
          {relationTypes.map((rt) => (
            <option key={rt} value={rt} className="bg-gray-900">
              {rt.replace(/_/g, ' ')}
            </option>
          ))}
        </select>
      )}

      {/* Group type input (for 'same_group' type) */}
      {needsGroupType && (
        <input
          type="text"
          value={rule.group_type || ''}
          onChange={(e) => onChange({ ...rule, group_type: e.target.value })}
          placeholder="Group type (e.g., organization)"
          className="px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all flex-1"
        />
      )}

      {/* Role input (for role-based types) */}
      {needsRole && (
        <input
          type="text"
          value={rule.role || ''}
          onChange={(e) => onChange({ ...rule, role: e.target.value })}
          placeholder="Role name (e.g., admin)"
          className="px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all flex-1"
        />
      )}

      {/* Spacer for types without extra fields */}
      {!needsRelation && !needsGroupType && !needsRole && <div className="flex-1" />}

      {/* Remove button */}
      <button
        onClick={onRemove}
        className="p-2 hover:bg-red-500/20 rounded-lg text-gray-400 hover:text-red-400 transition-colors"
        title="Remove rule"
      >
        <Trash2 className="w-4 h-4" />
      </button>
    </div>
  )
}
