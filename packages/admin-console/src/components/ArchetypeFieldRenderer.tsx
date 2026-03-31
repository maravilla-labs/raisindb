import { Info, AlertTriangle, Plus, HelpCircle, CheckCircle } from 'lucide-react'
import type { StaleFieldInfo } from '../api/translations'
import type { FieldSchema } from '../api/archetypes'
import StringField from './PropertyFields/StringField'
import NumberField from './PropertyFields/NumberField'
import BooleanField from './PropertyFields/BooleanField'
import SelectField from './PropertyFields/SelectField'
import ObjectField from './PropertyFields/ObjectField'
import ArrayField from './PropertyFields/ArrayField'
import DateField from './PropertyFields/DateField'
import SectionEditor from './PropertyFields/SectionEditor'
import ElementEditor from './PropertyFields/ElementEditor'
import CompositeFieldEditor from './PropertyFields/CompositeFieldEditor'
import MultipleFieldWrapper from './PropertyFields/MultipleFieldWrapper'

interface ArchetypeFieldRendererProps {
  field: FieldSchema & { $type: string; [key: string]: unknown }
  value: unknown
  error?: string
  onChange: (value: unknown) => void
  /** When true, only render if the field is marked translatable */
  translationMode?: boolean
  /** Original (source-language) value for translation reference */
  originalValue?: unknown
  /** Default language code shown in the "Original (en):" hint */
  defaultLanguage?: string
  repo?: string
  branch?: string
  /** Staleness status for this field */
  staleness?: 'fresh' | 'stale' | 'missing' | 'unknown' | null
  /** Detailed info about stale translation */
  staleInfo?: StaleFieldInfo | null
}

/** Extract the base properties shared by all FieldSchema variants */
function getBase(field: Record<string, unknown>): {
  name: string
  label: string
  required: boolean
  description?: string
  translatable?: boolean
  multiple?: boolean
  isHidden?: boolean
  defaultValue?: unknown
} {
  const base = (field.base ?? field) as Record<string, unknown>
  const name = (base.name ?? '') as string
  const label =
    (base.title as string) ??
    (base.label as string) ??
    name.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())

  return {
    name,
    label,
    required: !!(base.required as boolean | undefined),
    description: (base.description ?? base.help_text) as string | undefined,
    translatable: base.translatable as boolean | undefined,
    multiple: base.multiple as boolean | undefined,
    isHidden: base.is_hidden as boolean | undefined,
    defaultValue: base.default_value,
  }
}

/** Inline hint showing the original (source-language) value for a leaf field */
function OriginalValueHint({
  value,
  defaultLanguage,
}: {
  value: unknown
  defaultLanguage?: string
}) {
  if (value === undefined || value === null) return null
  const display =
    typeof value === 'object' ? JSON.stringify(value) : String(value)
  if (!display) return null
  return (
    <div className="mt-1.5 flex items-start gap-1.5 text-xs text-white/50">
      <Info className="w-3 h-3 mt-0.5 flex-shrink-0" />
      <span className="break-words min-w-0">
        Original{defaultLanguage ? ` (${defaultLanguage})` : ''}: {display}
      </span>
    </div>
  )
}

/** Field types that already handle arrays internally (no wrapping needed) */
const ARRAY_BASED_FIELDS = [
  'SectionField',
  'TagField',
  'ListingField',
  'CompositeField', // CompositeFieldEditor handles multiple prop internally
]

/** Get default value for a field type when adding new items in multiple mode */
function getDefaultValueForType(fieldType: string): unknown {
  switch (fieldType) {
    case 'NumberField':
      return undefined
    case 'BooleanField':
      return false
    case 'JsonObjectField':
      return {}
    default:
      return ''
  }
}

/** Staleness indicator badge */
function StalenessIndicator({
  staleness,
  staleInfo,
}: {
  staleness: 'fresh' | 'stale' | 'missing' | 'unknown' | null
  staleInfo?: StaleFieldInfo | null
}) {
  if (!staleness) return null

  switch (staleness) {
    case 'stale':
      return (
        <div className="flex items-center gap-1 px-2 py-0.5 bg-amber-500/20 border border-amber-500/40 rounded text-amber-300 text-xs">
          <AlertTriangle className="w-3 h-3" />
          <span>Original changed</span>
          {staleInfo?.translated_at && (
            <span className="text-amber-400/70 ml-1">
              (since {new Date(staleInfo.translated_at).toLocaleDateString()})
            </span>
          )}
        </div>
      )

    case 'missing':
      return (
        <div className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 border border-blue-500/40 rounded text-blue-300 text-xs">
          <Plus className="w-3 h-3" />
          <span>Needs translation</span>
        </div>
      )

    case 'unknown':
      return (
        <div className="flex items-center gap-1 px-2 py-0.5 bg-zinc-500/20 border border-zinc-500/40 rounded text-zinc-400 text-xs">
          <HelpCircle className="w-3 h-3" />
          <span>Staleness unknown</span>
        </div>
      )

    case 'fresh':
      return (
        <div className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 border border-green-500/40 rounded text-green-300 text-xs">
          <CheckCircle className="w-3 h-3" />
          <span>Up to date</span>
        </div>
      )

    default:
      return null
  }
}

export default function ArchetypeFieldRenderer({
  field,
  value,
  error,
  onChange,
  translationMode,
  originalValue,
  defaultLanguage,
  repo,
  branch,
  staleness,
  staleInfo,
}: ArchetypeFieldRendererProps) {
  const base = getBase(field)

  // Container fields always render in translation mode (children handle filtering)
  const CONTAINER_TYPES = ['CompositeField', 'SectionField', 'ElementField']
  const isContainerType = CONTAINER_TYPES.includes(field.$type)

  // In translation mode, skip non-translatable leaf fields
  // Container fields always render - their inner fields handle translatability
  if (translationMode && !base.translatable && !isContainerType) {
    return null
  }

  // Hidden fields are not rendered
  if (base.isHidden) return null

  const commonProps = {
    name: base.name,
    label: base.label,
    value: value as any,
    error,
    required: base.required,
    onChange: onChange as (val: any) => void,
  }

  const config = field.config as Record<string, unknown> | undefined

  // Container fields thread originalValue deeper — no hint at this level
  const isContainerField =
    field.$type === 'SectionField' ||
    field.$type === 'ElementField' ||
    field.$type === 'CompositeField'

  const fieldComponent = (() => {
    switch (field.$type) {
      case 'TextField':
        return (
          <StringField
            {...commonProps}
            placeholder={config?.placeholder as string}
          />
        )

      case 'RichTextField':
        return (
          <StringField
            {...commonProps}
            multiline
            placeholder={config?.placeholder as string}
          />
        )

      case 'NumberField':
        return (
          <NumberField
            {...commonProps}
            min={config?.min as number}
            max={config?.max as number}
            step={config?.step as number}
          />
        )

      case 'BooleanField':
        return <BooleanField {...commonProps} />

      case 'DateField':
        return <DateField {...commonProps} />

      case 'OptionsField': {
        const options: { label: string; value: string }[] = Array.isArray(
          config?.options
        )
          ? (config.options as { label: string; value: string }[])
          : []
        return (
          <SelectField
            {...commonProps}
            options={options}
          />
        )
      }

      case 'JsonObjectField':
        return <ObjectField {...commonProps} schema={config?.schema as Record<string, any> | undefined} />

      case 'TagField':
      case 'ListingField':
        return <ArrayField {...commonProps} />

      case 'MediaField':
        return (
          <StringField
            {...commonProps}
            placeholder="Media reference (path or URL)"
          />
        )

      case 'ReferenceField':
        return (
          <StringField
            {...commonProps}
            placeholder="Node reference path"
          />
        )

      case 'LocationField':
        return (
          <StringField
            {...commonProps}
            placeholder="Location value"
          />
        )

      case 'SectionField':
        return (
          <SectionEditor
            name={base.name}
            label={base.label}
            value={value as any}
            error={error}
            onChange={onChange}
            allowedElementTypes={
              (field as any).allowed_element_types as string[] | undefined
            }
            repo={repo}
            branch={branch}
            translationMode={translationMode}
            originalValue={originalValue}
            defaultLanguage={defaultLanguage}
          />
        )

      case 'ElementField':
        return (
          <ElementEditor
            name={base.name}
            label={base.label}
            value={value as any}
            error={error}
            onChange={onChange}
            elementType={(field as any).element_type as string | undefined}
            repo={repo}
            branch={branch}
            translationMode={translationMode}
            originalValue={originalValue}
            defaultLanguage={defaultLanguage}
          />
        )

      case 'CompositeField':
        return (
          <CompositeFieldEditor
            name={base.name}
            label={base.label}
            value={value as any}
            error={error}
            onChange={onChange}
            fields={(field as any).fields as FieldSchema[] | undefined}
            repo={repo}
            branch={branch}
            translationMode={translationMode}
            originalValue={originalValue}
            defaultLanguage={defaultLanguage}
            multiple={base.multiple}
          />
        )

      default:
        return (
          <StringField
            {...commonProps}
            placeholder={`Enter ${base.name}`}
          />
        )
    }
  })()

  // Check if we need to wrap with MultipleFieldWrapper
  // Only wrap leaf fields that don't already handle arrays internally
  const shouldWrapMultiple =
    base.multiple && !ARRAY_BASED_FIELDS.includes(field.$type)

  // Helper to render a single item for the wrapper
  const renderSingleItem = (
    itemValue: unknown,
    onItemChange: (v: unknown) => void
  ) => {
    const itemProps = {
      name: base.name,
      label: '', // Hide label, wrapper shows it
      value: itemValue as any,
      required: false, // Individual items don't need required marker
      onChange: onItemChange as (val: any) => void,
    }

    switch (field.$type) {
      case 'TextField':
        return (
          <StringField
            {...itemProps}
            placeholder={config?.placeholder as string}
          />
        )
      case 'RichTextField':
        return (
          <StringField
            {...itemProps}
            multiline
            placeholder={config?.placeholder as string}
          />
        )
      case 'NumberField':
        return (
          <NumberField
            {...itemProps}
            min={config?.min as number}
            max={config?.max as number}
            step={config?.step as number}
          />
        )
      case 'BooleanField':
        return <BooleanField {...itemProps} />
      case 'DateField':
        return <DateField {...itemProps} />
      case 'OptionsField': {
        const options: { label: string; value: string }[] = Array.isArray(
          config?.options
        )
          ? (config.options as { label: string; value: string }[])
          : []
        return <SelectField {...itemProps} options={options} />
      }
      case 'JsonObjectField':
        return (
          <ObjectField
            {...itemProps}
            schema={config?.schema as Record<string, any> | undefined}
          />
        )
      case 'MediaField':
        return (
          <StringField
            {...itemProps}
            placeholder="Media reference (path or URL)"
          />
        )
      case 'ReferenceField':
        return (
          <StringField {...itemProps} placeholder="Node reference path" />
        )
      case 'LocationField':
        return <StringField {...itemProps} placeholder="Location value" />
      default:
        return (
          <StringField {...itemProps} placeholder={`Enter ${base.name}`} />
        )
    }
  }

  const finalComponent = shouldWrapMultiple ? (
    <MultipleFieldWrapper
      name={base.name}
      label={base.label}
      value={value as unknown[]}
      onChange={onChange as (val: unknown[]) => void}
      translationMode={translationMode}
      originalValue={originalValue as unknown[]}
      error={error}
      getDefaultValue={() => getDefaultValueForType(field.$type)}
      renderItem={(itemValue, onItemChange) =>
        renderSingleItem(itemValue, onItemChange)
      }
    />
  ) : (
    fieldComponent
  )

  return (
    <div className="space-y-1">
      {/* Staleness indicator for translation mode */}
      {translationMode && staleness && (
        <div className="mb-1">
          <StalenessIndicator staleness={staleness} staleInfo={staleInfo} />
        </div>
      )}
      {finalComponent}
      {base.description && (
        <p className="text-xs text-zinc-500">{base.description}</p>
      )}
      {/* Show original-value hint for leaf fields in translation mode */}
      {translationMode && !isContainerField && !shouldWrapMultiple && originalValue !== undefined && (
        <OriginalValueHint
          value={originalValue}
          defaultLanguage={defaultLanguage}
        />
      )}
    </div>
  )
}
