/**
 * Function Properties Form Component
 *
 * Form for editing core function properties (name, title, language, entry_file, execution_mode, enabled).
 * Used within the RaisinFunctionNodeTypeEditor.
 */

import { useState } from 'react'
import { ToggleLeft, ToggleRight, Braces, Check, ChevronDown, ChevronRight, Globe, Cpu, KeyRound, Mail } from 'lucide-react'
import Editor from '@monaco-editor/react'
import type { FunctionProperties, FunctionLanguage, ExecutionMode, NetworkPolicy, ResourceLimits, SecretPolicy, EmailPolicy } from '../../types'

/** Default network policy values */
const DEFAULT_NETWORK_POLICY: NetworkPolicy = {
  http_enabled: true,
  allowed_urls: [],
  request_timeout_ms: 30000,
  max_concurrent_requests: 10,
}

/**
 * Defaults for the two deny-by-default policies.
 *
 * Both start `enabled: false` with an empty list, matching the server: an omitted
 * block grants nothing. Unlike the network policy — whose `http_enabled` default
 * is `true` — there is no permissive default to preserve here, so the form must
 * not invent one.
 */
const DEFAULT_SECRET_POLICY: SecretPolicy = {
  enabled: false,
  allowed_names: [],
}

const DEFAULT_EMAIL_POLICY: EmailPolicy = {
  enabled: false,
  allowed_recipients: [],
}

/** Default resource limits values */
const DEFAULT_RESOURCE_LIMITS: ResourceLimits = {
  timeout_ms: 30000,
  max_memory_bytes: 134217728, // 128MB
  max_stack_bytes: 1048576, // 1MB
}

interface CollapsibleSectionProps {
  title: string
  icon: React.ReactNode
  defaultOpen?: boolean
  children: React.ReactNode
}

function CollapsibleSection({ title, icon, defaultOpen = false, children }: CollapsibleSectionProps) {
  const [isOpen, setIsOpen] = useState(defaultOpen)

  return (
    <div className="border border-white/10 rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        className="w-full flex items-center gap-2 px-3 py-2 bg-white/5 hover:bg-white/10 transition-colors text-left"
      >
        {isOpen ? <ChevronDown className="w-4 h-4 text-gray-400" /> : <ChevronRight className="w-4 h-4 text-gray-400" />}
        {icon}
        <span className="text-sm font-medium text-gray-200">{title}</span>
      </button>
      {isOpen && (
        <div className="p-3 border-t border-white/10">
          {children}
        </div>
      )}
    </div>
  )
}

interface FieldProps {
  label: string
  children: React.ReactNode
  hint?: string
}

function Field({ label, children, hint }: FieldProps) {
  return (
    <div className="mb-4">
      <label className="block text-xs text-gray-400 mb-1">{label}</label>
      {children}
      {hint && <p className="text-xs text-gray-500 mt-1">{hint}</p>}
    </div>
  )
}

/** Enabled/disabled pill, shared by the two policy sections. */
function PolicyToggle({
  label,
  enabled,
  disabled,
  onToggle,
}: {
  label: string
  enabled: boolean
  disabled: boolean
  onToggle: () => void
}) {
  return (
    <div className="flex items-center justify-between">
      <label className="text-xs text-gray-400">{label}</label>
      <button
        type="button"
        onClick={onToggle}
        disabled={disabled}
        className={`flex items-center gap-1.5 px-2 py-1 rounded text-xs transition-colors
          ${enabled
            ? 'bg-green-500/20 text-green-300 hover:bg-green-500/30'
            : 'bg-red-500/20 text-red-300 hover:bg-red-500/30'
          }
          disabled:opacity-50 disabled:cursor-not-allowed
        `}
      >
        {enabled ? (
          <>
            <ToggleRight className="w-3 h-3" />
            Enabled
          </>
        ) : (
          <>
            <ToggleLeft className="w-3 h-3" />
            Disabled
          </>
        )}
      </button>
    </div>
  )
}

/**
 * Newline-separated allow-list editor, shared by the two policy sections.
 *
 * Keeps empty lines while typing (so a newline is not eaten mid-edit) and only
 * prunes them on blur — the same two-handler pattern the allowed_urls field uses.
 */
function PolicyList({
  value,
  placeholder,
  disabled,
  onChange,
}: {
  value: string[]
  placeholder: string
  disabled: boolean
  onChange: (next: string[]) => void
}) {
  return (
    <textarea
      value={value.join('\n')}
      onChange={(e) => onChange(e.target.value.split('\n'))}
      onBlur={(e) => onChange(e.target.value.split('\n').filter((v) => v.trim()))}
      placeholder={placeholder}
      disabled={disabled}
      rows={3}
      className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-gray-200 placeholder:text-gray-600 focus:outline-none focus:border-blue-500/50 disabled:opacity-50 font-mono text-xs resize-none"
    />
  )
}

export interface FunctionPropertiesFormProps {
  properties: Partial<FunctionProperties>
  onChange: (properties: Partial<FunctionProperties>) => void
  disabled?: boolean
  /** Called when user wants to open the schema editor */
  onOpenSchemaEditor?: (schemaType: 'input' | 'output') => void
}

export function FunctionPropertiesForm({
  properties,
  onChange,
  disabled = false,
  onOpenSchemaEditor,
}: FunctionPropertiesFormProps) {
  const handleChange = <K extends keyof FunctionProperties>(
    key: K,
    value: FunctionProperties[K]
  ) => {
    onChange({
      ...properties,
      [key]: value,
    })
  }

  const inputClass = `w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-white placeholder-gray-500
    focus:outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500
    disabled:opacity-50 disabled:cursor-not-allowed`

  const selectClass = `w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm text-white
    focus:outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500
    disabled:opacity-50 disabled:cursor-not-allowed`

  return (
    <div className="space-y-4">
      {/* Name (read-only) */}
      <Field label="Name">
        <input
          type="text"
          value={properties.name || ''}
          readOnly
          disabled
          className={inputClass}
        />
      </Field>

      {/* Title */}
      <Field label="Title">
        <input
          type="text"
          value={properties.title || ''}
          onChange={(e) => handleChange('title', e.target.value)}
          placeholder="Display title"
          disabled={disabled}
          className={inputClass}
        />
      </Field>

      {/* Description */}
      <Field
        label="Description"
        hint="Describe what this function does. This helps AI agents understand when and how to use this tool."
      >
        <div className="border border-white/10 rounded overflow-hidden">
          <Editor
            height="120px"
            language="markdown"
            theme="vs-dark"
            value={properties.description || ''}
            onChange={(value) => handleChange('description', value || '')}
            options={{
              minimap: { enabled: false },
              fontSize: 12,
              lineNumbers: 'off',
              wordWrap: 'on',
              scrollBeyondLastLine: false,
              padding: { top: 8, bottom: 8 },
              automaticLayout: true,
              readOnly: disabled,
              folding: false,
              lineDecorationsWidth: 8,
              renderLineHighlight: 'none',
              scrollbar: {
                verticalScrollbarSize: 8,
                horizontalScrollbarSize: 8,
              },
            }}
          />
        </div>
      </Field>

      {/* Language */}
      <Field label="Language">
        <select
          value={properties.language || 'javascript'}
          onChange={(e) => handleChange('language', e.target.value as FunctionLanguage)}
          disabled={disabled}
          className={selectClass}
        >
          <option value="javascript">JavaScript</option>
          <option value="starlark">Python (Starlark)</option>
          <option value="sql">SQL</option>
        </select>
      </Field>

      {/* Entry File */}
      <Field
        label="Entry File"
        hint={`Format: filename:functionName (e.g., ${
          properties.language === 'starlark' ? 'index.py:handler' : 'index.js:handler'
        })`}
      >
        <input
          type="text"
          value={properties.entry_file || ''}
          onChange={(e) => handleChange('entry_file', e.target.value)}
          placeholder={properties.language === 'starlark' ? 'index.py:handler' : 'index.js:handler'}
          disabled={disabled}
          className={inputClass}
        />
      </Field>

      {/* Execution Mode */}
      <Field label="Execution Mode">
        <select
          value={properties.execution_mode || 'async'}
          onChange={(e) => handleChange('execution_mode', e.target.value as ExecutionMode)}
          disabled={disabled}
          className={selectClass}
        >
          <option value="async">Async</option>
          <option value="sync">Sync</option>
          <option value="both">Both</option>
        </select>
      </Field>

      {/* Enabled */}
      <Field label="Enabled">
        <button
          type="button"
          onClick={() => handleChange('enabled', !(properties.enabled !== false))}
          disabled={disabled}
          className={`flex items-center gap-2 px-3 py-1.5 rounded text-sm transition-colors
            ${properties.enabled !== false
              ? 'bg-green-500/20 text-green-300 hover:bg-green-500/30'
              : 'bg-red-500/20 text-red-300 hover:bg-red-500/30'
            }
            disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:bg-transparent
          `}
        >
          {properties.enabled !== false ? (
            <>
              <ToggleRight className="w-4 h-4" />
              Enabled
            </>
          ) : (
            <>
              <ToggleLeft className="w-4 h-4" />
              Disabled
            </>
          )}
        </button>
      </Field>

      {/* Input Schema */}
      <Field label="Input Schema" hint="JSON Schema for validating function input (used by AI agents)">
        <button
          type="button"
          onClick={() => onOpenSchemaEditor?.('input')}
          disabled={disabled}
          className={`flex items-center gap-2 px-3 py-1.5 rounded text-sm transition-colors
            ${properties.input_schema
              ? 'bg-blue-500/20 text-blue-300 hover:bg-blue-500/30'
              : 'bg-white/5 text-gray-400 hover:bg-white/10 hover:text-gray-300'
            }
            disabled:opacity-50 disabled:cursor-not-allowed
          `}
        >
          <Braces className="w-4 h-4" />
          {properties.input_schema ? (
            <>
              <Check className="w-3 h-3" />
              Edit Schema
            </>
          ) : (
            'Define Schema'
          )}
        </button>
      </Field>

      {/* Output Schema */}
      <Field label="Output Schema" hint="JSON Schema for validating function output (used by AI agents)">
        <button
          type="button"
          onClick={() => onOpenSchemaEditor?.('output')}
          disabled={disabled}
          className={`flex items-center gap-2 px-3 py-1.5 rounded text-sm transition-colors
            ${properties.output_schema
              ? 'bg-purple-500/20 text-purple-300 hover:bg-purple-500/30'
              : 'bg-white/5 text-gray-400 hover:bg-white/10 hover:text-gray-300'
            }
            disabled:opacity-50 disabled:cursor-not-allowed
          `}
        >
          <Braces className="w-4 h-4" />
          {properties.output_schema ? (
            <>
              <Check className="w-3 h-3" />
              Edit Schema
            </>
          ) : (
            'Define Schema'
          )}
        </button>
      </Field>

      {/* Network Policy Section */}
      <CollapsibleSection
        title="Network Policy"
        icon={<Globe className="w-4 h-4 text-blue-400" />}
        defaultOpen={!!(properties.network_policy?.allowed_urls?.length)}
      >
        <div className="space-y-3">
          {/* HTTP Enabled */}
          <div className="flex items-center justify-between">
            <label className="text-xs text-gray-400">HTTP Enabled</label>
            <button
              type="button"
              onClick={() => {
                const current = properties.network_policy || DEFAULT_NETWORK_POLICY
                onChange({
                  ...properties,
                  network_policy: {
                    ...current,
                    http_enabled: !current.http_enabled,
                  },
                })
              }}
              disabled={disabled}
              className={`flex items-center gap-1.5 px-2 py-1 rounded text-xs transition-colors
                ${(properties.network_policy?.http_enabled ?? true)
                  ? 'bg-green-500/20 text-green-300 hover:bg-green-500/30'
                  : 'bg-red-500/20 text-red-300 hover:bg-red-500/30'
                }
                disabled:opacity-50 disabled:cursor-not-allowed
              `}
            >
              {(properties.network_policy?.http_enabled ?? true) ? (
                <>
                  <ToggleRight className="w-3 h-3" />
                  Enabled
                </>
              ) : (
                <>
                  <ToggleLeft className="w-3 h-3" />
                  Disabled
                </>
              )}
            </button>
          </div>

          {/* Allowed URLs */}
          <div>
            <label className="block text-xs text-gray-400 mb-1">
              Allowed URLs
            </label>
            <p className="text-xs text-gray-500 mb-2">
              One URL pattern per line. Use <code className="bg-white/10 px-1 rounded">**</code> to match any path including slashes,{' '}
              <code className="bg-white/10 px-1 rounded">*</code> for single path segment.
              <br />
              Examples: <code className="bg-white/10 px-1 rounded">https://api.example.com/**</code> matches all paths
            </p>
            <textarea
              value={(properties.network_policy?.allowed_urls || []).join('\n')}
              onChange={(e) => {
                // Keep all lines during editing (including empty ones for newlines)
                const urls = e.target.value.split('\n')
                const current = properties.network_policy || DEFAULT_NETWORK_POLICY
                onChange({
                  ...properties,
                  network_policy: {
                    ...current,
                    allowed_urls: urls,
                  },
                })
              }}
              onBlur={(e) => {
                // Clean up empty lines on blur
                const urls = e.target.value.split('\n').filter((u) => u.trim())
                const current = properties.network_policy || DEFAULT_NETWORK_POLICY
                onChange({
                  ...properties,
                  network_policy: {
                    ...current,
                    allowed_urls: urls,
                  },
                })
              }}
              placeholder="https://api.example.com/**&#10;https://geocoding-api.open-meteo.com/**"
              disabled={disabled}
              rows={4}
              className={`${inputClass} font-mono text-xs resize-none`}
            />
          </div>

          {/* Request Timeout */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-gray-400 w-32">Request Timeout</label>
            <input
              type="number"
              value={properties.network_policy?.request_timeout_ms ?? DEFAULT_NETWORK_POLICY.request_timeout_ms}
              onChange={(e) => {
                const current = properties.network_policy || DEFAULT_NETWORK_POLICY
                onChange({
                  ...properties,
                  network_policy: {
                    ...current,
                    request_timeout_ms: parseInt(e.target.value, 10) || 30000,
                  },
                })
              }}
              disabled={disabled}
              min={1000}
              max={300000}
              className={`${inputClass} w-24`}
            />
            <span className="text-xs text-gray-500">ms</span>
          </div>

          {/* Max Concurrent Requests */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-gray-400 w-32">Max Concurrent</label>
            <input
              type="number"
              value={properties.network_policy?.max_concurrent_requests ?? DEFAULT_NETWORK_POLICY.max_concurrent_requests}
              onChange={(e) => {
                const current = properties.network_policy || DEFAULT_NETWORK_POLICY
                onChange({
                  ...properties,
                  network_policy: {
                    ...current,
                    max_concurrent_requests: parseInt(e.target.value, 10) || 10,
                  },
                })
              }}
              disabled={disabled}
              min={1}
              max={100}
              className={`${inputClass} w-24`}
            />
            <span className="text-xs text-gray-500">requests</span>
          </div>
        </div>
      </CollapsibleSection>

      {/* Secret Policy Section */}
      <CollapsibleSection
        title="Secret Policy"
        icon={<KeyRound className="w-4 h-4 text-amber-400" />}
        defaultOpen={!!(properties.secret_policy?.allowed_names?.length)}
      >
        <div className="space-y-3">
          <p className="text-xs text-gray-500">
            Gates <code className="bg-white/10 px-1 rounded">raisin.secrets.*</code>. Deny-by-default:
            left off, or on with an empty list, it grants <strong>no access at all</strong>.
          </p>

          <PolicyToggle
            label="Secrets Enabled"
            enabled={properties.secret_policy?.enabled ?? false}
            disabled={disabled}
            onToggle={() => {
              const current = properties.secret_policy || DEFAULT_SECRET_POLICY
              onChange({
                ...properties,
                secret_policy: { ...current, enabled: !current.enabled },
              })
            }}
          />

          <div>
            <label className="block text-xs text-gray-400 mb-1">Allowed Names</label>
            <p className="text-xs text-gray-500 mb-2">
              One glob per line, e.g. <code className="bg-white/10 px-1 rounded">stripe/*</code>.
              Namespace by prefix so one function cannot read another integration&apos;s credentials.
            </p>
            <PolicyList
              value={properties.secret_policy?.allowed_names || []}
              placeholder={'stripe/platform-key\nstripe/account-id/*'}
              disabled={disabled}
              onChange={(allowed_names) => {
                const current = properties.secret_policy || DEFAULT_SECRET_POLICY
                onChange({ ...properties, secret_policy: { ...current, allowed_names } })
              }}
            />
            {properties.secret_policy?.enabled &&
              !(properties.secret_policy?.allowed_names || []).some((n) => n.trim()) && (
                <p className="text-xs text-amber-300/80 mt-1">
                  Enabled with an empty list still denies everything.
                </p>
              )}
            {(properties.secret_policy?.allowed_names || []).some((n) => n.trim() === '*') && (
              <p className="text-xs text-red-300/80 mt-1">
                <code className="bg-white/10 px-1 rounded">*</code> grants every secret in the
                repository. A function with this and HTTP access can exfiltrate all of them.
              </p>
            )}
          </div>
        </div>
      </CollapsibleSection>

      {/* Email Policy Section */}
      <CollapsibleSection
        title="Email Policy"
        icon={<Mail className="w-4 h-4 text-sky-400" />}
        defaultOpen={!!(properties.email_policy?.allowed_recipients?.length)}
      >
        <div className="space-y-3">
          <p className="text-xs text-gray-500">
            Gates <code className="bg-white/10 px-1 rounded">raisin.email.send</code>.
            Deny-by-default. Mail leaves from the tenant&apos;s verified sending domain, so an
            ungated function can send authenticated mail <em>as the tenant</em> — the secret policy
            governs who may read the provider credential, not who may be written to.
          </p>

          <PolicyToggle
            label="Email Enabled"
            enabled={properties.email_policy?.enabled ?? false}
            disabled={disabled}
            onToggle={() => {
              const current = properties.email_policy || DEFAULT_EMAIL_POLICY
              onChange({
                ...properties,
                email_policy: { ...current, enabled: !current.enabled },
              })
            }}
          />

          <div>
            <label className="block text-xs text-gray-400 mb-1">Allowed Recipients</label>
            <p className="text-xs text-gray-500 mb-2">
              One recipient <strong>domain</strong> per line, e.g.{' '}
              <code className="bg-white/10 px-1 rounded">example.com</code> — a domain, not a full
              address. Or <code className="bg-white/10 px-1 rounded">*</code> for any address.
            </p>
            <p className="text-xs text-gray-500 mb-2">
              <strong className="text-gray-400">Mail to your users needs</strong>{' '}
              <code className="bg-white/10 px-1 rounded">*</code>. A sign-in link, a registration
              confirmation or a password reset goes to whatever address the person typed, and you
              cannot know those domains in advance — the built-in{' '}
              <code className="bg-white/10 px-1 rounded">send-magic-link</code> function ships with{' '}
              <code className="bg-white/10 px-1 rounded">*</code> for exactly this reason. Use a
              domain list for mail with a known audience: staff notifications, internal alerts, or a
              deployment where only your own people may sign in.
            </p>
            <PolicyList
              value={properties.email_policy?.allowed_recipients || []}
              placeholder={'example.com\npartners.example.org'}
              disabled={disabled}
              onChange={(allowed_recipients) => {
                const current = properties.email_policy || DEFAULT_EMAIL_POLICY
                onChange({ ...properties, email_policy: { ...current, allowed_recipients } })
              }}
            />
            {properties.email_policy?.enabled &&
              !(properties.email_policy?.allowed_recipients || []).some((r) => r.trim()) && (
                <p className="text-xs text-amber-300/80 mt-1">
                  Enabled with an empty list still denies everything.
                </p>
              )}
            {/* Informational, NOT red. `*` is the correct setting for any mail
                addressed to end users, so styling it as an error tells the
                operator the right answer is wrong and invites them to break
                sign-in by narrowing it. */}
            {(properties.email_policy?.allowed_recipients || []).some((r) => r.trim() === '*') && (
              <p className="text-xs text-sky-300/80 mt-1">
                <code className="bg-white/10 px-1 rounded">*</code> allows mail to any address —
                correct for user-facing mail, worth narrowing if this function only writes to a
                known audience.
              </p>
            )}
          </div>
        </div>
      </CollapsibleSection>

      {/* Resource Limits Section */}
      <CollapsibleSection
        title="Resource Limits"
        icon={<Cpu className="w-4 h-4 text-orange-400" />}
      >
        <div className="space-y-3">
          {/* Timeout */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-gray-400 w-32">Timeout</label>
            <input
              type="number"
              value={properties.resource_limits?.timeout_ms ?? DEFAULT_RESOURCE_LIMITS.timeout_ms}
              onChange={(e) => {
                const current = properties.resource_limits || DEFAULT_RESOURCE_LIMITS
                onChange({
                  ...properties,
                  resource_limits: {
                    ...current,
                    timeout_ms: parseInt(e.target.value, 10) || 30000,
                  },
                })
              }}
              disabled={disabled}
              min={1000}
              max={300000}
              className={`${inputClass} w-24`}
            />
            <span className="text-xs text-gray-500">ms</span>
          </div>

          {/* Max Memory */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-gray-400 w-32">Max Memory</label>
            <input
              type="number"
              value={Math.round((properties.resource_limits?.max_memory_bytes ?? DEFAULT_RESOURCE_LIMITS.max_memory_bytes) / (1024 * 1024))}
              onChange={(e) => {
                const current = properties.resource_limits || DEFAULT_RESOURCE_LIMITS
                const mbValue = parseInt(e.target.value, 10) || 128
                onChange({
                  ...properties,
                  resource_limits: {
                    ...current,
                    max_memory_bytes: mbValue * 1024 * 1024,
                  },
                })
              }}
              disabled={disabled}
              min={1}
              max={1024}
              className={`${inputClass} w-24`}
            />
            <span className="text-xs text-gray-500">MB</span>
          </div>

          {/* Max Stack */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-gray-400 w-32">Max Stack</label>
            <input
              type="number"
              value={Math.round((properties.resource_limits?.max_stack_bytes ?? DEFAULT_RESOURCE_LIMITS.max_stack_bytes) / (1024 * 1024))}
              onChange={(e) => {
                const current = properties.resource_limits || DEFAULT_RESOURCE_LIMITS
                const mbValue = parseInt(e.target.value, 10) || 1
                onChange({
                  ...properties,
                  resource_limits: {
                    ...current,
                    max_stack_bytes: mbValue * 1024 * 1024,
                  },
                })
              }}
              disabled={disabled}
              min={1}
              max={16}
              className={`${inputClass} w-24`}
            />
            <span className="text-xs text-gray-500">MB</span>
          </div>
        </div>
      </CollapsibleSection>
    </div>
  )
}
