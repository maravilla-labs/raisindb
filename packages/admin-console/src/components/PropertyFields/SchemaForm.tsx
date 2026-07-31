// SPDX-License-Identifier: BSL-1.1

import { useMemo, useState } from 'react'
import { KeyRound, CheckCircle2, Eye, EyeOff } from 'lucide-react'
import StringField from './StringField'
import NumberField from './NumberField'
import BooleanField from './BooleanField'
import SelectField from './SelectField'
import ObjectField from './ObjectField'
import ArrayField from './ArrayField'
import DateField from './DateField'
import {
  getSchemaType,
  getSchemaLabel,
  getSchemaPlaceholder,
  getSchemaDescription,
  getSchemaStructure,
  getSchemaItems,
  getEnumOptions,
  getFieldGroup,
  getFieldOrder,
  isSchemaRequired,
  isSecretField,
} from '../../utils/propertySchema'

/** One resolved property of a NodeType. */
export interface SchemaProperty {
  name?: string
  [key: string]: any
}

export interface SchemaFormProps {
  /** Resolved properties, e.g. `ResolvedNodeType.resolved_properties`. */
  properties: SchemaProperty[]
  /** Current non-secret values, keyed by property name. */
  values: Record<string, any>
  /**
   * Names of the secret fields that already have a stored value. Secret values
   * themselves are never sent to the browser — only which ones are set.
   */
  secretsSet?: string[]
  /** Pending secret edits, keyed by name. `null` means "clear this field". */
  secretDrafts?: Record<string, string | null>
  onChange: (name: string, value: any) => void
  onSecretChange?: (name: string, value: string | null) => void
  disabled?: boolean
}

const labelCls = 'block text-xs font-medium text-zinc-400 mb-1'
const fieldCls =
  'w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-white/40 focus:outline-none focus:ring-2 focus:ring-primary-500'

/**
 * Renders a form from a NodeType's resolved properties.
 *
 * Extracted from `NodeTypeAwareEditor` so connector configuration can be
 * schema-driven without duplicating the type switch. The one branch that editor
 * does not have is {@link isSecretField}: a secret renders write-only, showing
 * whether a value is stored but never the value itself.
 *
 * Presentation hints (label/description/placeholder/enum/group/order) are read
 * through the `propertySchema` helpers, which fall back to `meta` — the server
 * drops the top-level equivalents when it deserializes a NodeType.
 */
export default function SchemaForm({
  properties,
  values,
  secretsSet = [],
  secretDrafts = {},
  onChange,
  onSecretChange,
  disabled,
}: SchemaFormProps) {
  // Group, then order within each group. Fields with no `order` sink to the end
  // in their authored sequence.
  const groups = useMemo(() => {
    const named = properties.filter((p) => p.name)
    const byGroup = new Map<string, SchemaProperty[]>()
    for (const p of named) {
      const g = getFieldGroup(p) || ''
      if (!byGroup.has(g)) byGroup.set(g, [])
      byGroup.get(g)!.push(p)
    }
    for (const list of byGroup.values()) {
      list.sort((a, b) => getFieldOrder(a) - getFieldOrder(b))
    }
    return [...byGroup.entries()]
  }, [properties])

  if (properties.length === 0) return null

  return (
    <div className="space-y-5">
      {groups.map(([group, props]) => (
        <div key={group || '_'} className="space-y-3">
          {group && (
            <div className="text-[10px] uppercase tracking-wider text-zinc-500">{group}</div>
          )}
          {props.map((schema) => {
            const name = schema.name as string
            return isSecretField(schema) ? (
              <SecretInput
                key={name}
                name={name}
                schema={schema}
                isSet={secretsSet.includes(name)}
                draft={secretDrafts[name]}
                onChange={(v) => onSecretChange?.(name, v)}
                disabled={disabled}
              />
            ) : (
              <PlainField
                key={name}
                name={name}
                schema={schema}
                value={values[name]}
                onChange={(v) => onChange(name, v)}
                disabled={disabled}
              />
            )
          })}
        </div>
      ))}
    </div>
  )
}

function PlainField({
  name,
  schema,
  value,
  onChange,
  disabled,
}: {
  name: string
  schema: SchemaProperty
  value: any
  onChange: (v: any) => void
  disabled?: boolean
}) {
  const type = getSchemaType(schema)
  const label = getSchemaLabel(name, schema)
  const description = getSchemaDescription(schema)
  const placeholder = getSchemaPlaceholder(schema)
  const required = isSchemaRequired(schema)
  const options = getEnumOptions(schema)

  // The shared PropertyFields components take (name, label, value, onChange)
  // and render no description, so help text is rendered here alongside them.
  const base = { name, label, required }

  let control: JSX.Element
  if (options && options.length > 0) {
    // An enumerated field is a select regardless of its underlying type.
    control = (
      <SelectField {...base} value={value ?? ''} options={options} onChange={onChange} />
    )
  } else {
    switch (type) {
      case 'number':
        control = <NumberField {...base} value={value} onChange={onChange} />
        break
      case 'boolean':
        control = <BooleanField {...base} value={Boolean(value)} onChange={onChange} />
        break
      case 'date':
        control = (
          <DateField {...base} value={value ?? ''} placeholder={placeholder} onChange={onChange} />
        )
        break
      case 'object':
        control = (
          <ObjectField
            {...base}
            value={value ?? {}}
            schema={getSchemaStructure(schema)}
            onChange={onChange}
          />
        )
        break
      case 'array':
        control = (
          <ArrayField
            {...base}
            value={Array.isArray(value) ? value : []}
            itemType={getSchemaItems(schema)}
            onChange={onChange}
          />
        )
        break
      default:
        control = (
          <StringField
            {...base}
            value={value ?? ''}
            placeholder={placeholder}
            multiline={Boolean(schema?.multiline ?? schema?.meta?.multiline)}
            onChange={onChange}
          />
        )
    }
  }

  return (
    <div className={disabled ? 'opacity-60 pointer-events-none' : undefined}>
      {control}
      {description && <p className="mt-1 text-xs text-zinc-500">{description}</p>}
    </div>
  )
}

/**
 * Write-only input for a secret.
 *
 * Deliberately never receives a value: the server does not send one. It shows
 * whether a secret is stored, lets the operator type a replacement, and offers
 * an explicit clear. Leaving it blank keeps whatever is stored — so an edit form
 * never has to round-trip a value the operator was not shown.
 *
 * The reveal toggle shows only what was typed **in this session**. A stored
 * secret can never be revealed, because the browser has never received it —
 * that is the point of the write-only contract, not a UI limitation. The toggle
 * exists so an operator can check a long app password they just pasted, and it
 * resets to hidden whenever the field is cleared.
 */
function SecretInput({
  name,
  schema,
  isSet,
  draft,
  onChange,
  disabled,
}: {
  name: string
  schema: SchemaProperty
  isSet: boolean
  draft?: string | null
  onChange: (v: string | null) => void
  disabled?: boolean
}) {
  const label = getSchemaLabel(name, schema)
  const description = getSchemaDescription(schema)
  const cleared = draft === null
  const [reveal, setReveal] = useState(false)
  // Only a typed value can be revealed; there is nothing else to show.
  const canReveal = typeof draft === 'string' && draft.length > 0 && !cleared

  return (
    <div>
      <label className={labelCls}>
        <span className="inline-flex items-center gap-1">
          <KeyRound className="w-3 h-3" /> {label}
          {isSchemaRequired(schema) && !isSet && <span className="text-red-400">*</span>}
        </span>
      </label>
      <div className="relative">
        <input
          type={reveal && canReveal ? 'text' : 'password'}
          autoComplete="new-password"
          className={`${fieldCls} pr-10`}
          value={typeof draft === 'string' ? draft : ''}
          disabled={disabled || cleared}
          placeholder={isSet ? '•••••••• (stored — leave blank to keep)' : 'Enter a value'}
          onChange={(e) => onChange(e.target.value)}
        />
        {canReveal && (
          <button
            type="button"
            onClick={() => setReveal((v) => !v)}
            title={reveal ? 'Hide' : 'Show what you typed'}
            aria-label={reveal ? 'Hide secret' : 'Show secret'}
            className="absolute inset-y-0 right-0 px-3 flex items-center text-zinc-500 hover:text-zinc-300"
          >
            {reveal ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
          </button>
        )}
      </div>
      <div className="mt-1 flex items-center gap-3 text-xs">
        {isSet && !cleared && (
          <span className="inline-flex items-center gap-1 text-green-400">
            <CheckCircle2 className="w-3 h-3" /> stored (never displayed)
          </span>
        )}
        {cleared && <span className="text-amber-400">will be cleared on save</span>}
        {isSet && !cleared && (
          <button
            type="button"
            className="text-zinc-500 hover:text-red-400 underline"
            onClick={() => {
              setReveal(false)
              onChange(null)
            }}
            disabled={disabled}
          >
            Clear
          </button>
        )}
        {cleared && (
          <button
            type="button"
            className="text-zinc-500 hover:text-zinc-300 underline"
            onClick={() => onChange('')}
            disabled={disabled}
          >
            Undo
          </button>
        )}
      </div>
      {description && <p className="mt-1 text-xs text-zinc-500">{description}</p>}
    </div>
  )
}
