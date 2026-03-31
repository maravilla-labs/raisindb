import { useEffect, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  Plus,
  Trash2,
  Edit3,
  X,
  Layers,
} from 'lucide-react'
import type { Permission } from '../api/roles'
import { nodeTypesApi } from '../api/nodetypes'
import TagSelector from './TagSelector'
import { ConditionBuilder } from './ConditionBuilder'

interface PermissionEditorProps {
  permissions: Permission[]
  onChange: (permissions: Permission[]) => void
  nodeTypeOptions?: string[]
  repo?: string
  branch?: string
}

type PermissionDraft = {
  workspace: string
  branch_pattern: string
  path: string
  operations: string[]
  node_types: string[]
  fields: string[]
  except_fields: string[]
  condition: string
}

const OPERATIONS = ['create', 'read', 'update', 'delete', 'translate', 'relate', 'unrelate']

function formatOperationLabel(operation: string) {
  return operation.charAt(0).toUpperCase() + operation.slice(1)
}

// Validate a path pattern for glob-style matching.
// Returns error message if invalid, null if valid.
function validatePathPattern(pattern: string): string | null {
  // Must start with /
  if (!pattern.startsWith('/')) {
    return 'Path pattern must start with /'
  }

  // No double slashes (except for /**/ which is valid)
  if (pattern.includes('///')) {
    return 'Path cannot contain triple slashes'
  }

  // Check for empty path (just /)
  if (pattern === '/') {
    return null // Root path is valid (matches all)
  }

  // Valid characters: alphanumeric, underscore, hyphen, slash, asterisk, question mark
  const validChars = /^[a-zA-Z0-9_\-\/\*\?]+$/
  if (!validChars.test(pattern)) {
    return 'Invalid characters in path pattern. Allowed: letters, numbers, -, _, /, *, ?'
  }

  // Check for valid glob patterns (basic validation)
  // Asterisks should be either * or ** but not *** or more
  if (/\*{3,}/.test(pattern)) {
    return 'Invalid pattern: use * or ** (not *** or more)'
  }

  return null
}

function createDraftFromPermission(permission?: Permission): PermissionDraft {
  return {
    workspace: permission?.workspace ?? '',
    branch_pattern: permission?.branch_pattern ?? '',
    path: permission?.path ?? '',
    operations: permission?.operations ? [...permission.operations] : [],
    node_types: permission?.node_types ? [...permission.node_types] : [],
    fields: permission?.fields ? [...permission.fields] : [],
    except_fields: permission?.except_fields ? [...permission.except_fields] : [],
    condition: permission?.condition ?? '',
  }
}

function normalizePermission(draft: PermissionDraft): Permission {
  const permission: Permission = {
    path: draft.path.trim(),
    operations: [...draft.operations],
  }
  // Include workspace if non-empty (empty defaults to "*" meaning all)
  if (draft.workspace && draft.workspace.trim()) {
    permission.workspace = draft.workspace.trim()
  }
  // Include branch_pattern if non-empty (empty defaults to "*" meaning all)
  if (draft.branch_pattern && draft.branch_pattern.trim()) {
    permission.branch_pattern = draft.branch_pattern.trim()
  }
  if (draft.node_types.length > 0) {
    permission.node_types = draft.node_types
  }
  if (draft.fields.length > 0) {
    permission.fields = draft.fields
  }
  if (draft.except_fields.length > 0) {
    permission.except_fields = draft.except_fields
  }
  if (draft.condition && draft.condition.trim()) {
    permission.condition = draft.condition.trim()
  }
  return permission
}

interface PermissionDialogProps {
  open: boolean
  title: string
  permission?: Permission
  onCancel: () => void
  onSave: (permission: Permission) => void
  nodeTypeOptions: string[]
  repo?: string
  branch?: string
}

function PermissionDialog({
  open,
  title,
  permission,
  onCancel,
  onSave,
  nodeTypeOptions,
  repo,
  branch,
}: PermissionDialogProps) {
  const nodeTypeSet = useMemo(() => new Set(nodeTypeOptions), [nodeTypeOptions])
  const [draft, setDraft] = useState<PermissionDraft>(createDraftFromPermission(permission))
  const [activeTab, setActiveTab] = useState<'details' | 'conditions'>('details')
  const [formError, setFormError] = useState<string | null>(null)
  const [pathError, setPathError] = useState<string | null>(null)
  const [availableFields, setAvailableFields] = useState<string[]>([])

  useEffect(() => {
    if (open) {
      setDraft(createDraftFromPermission(permission))
      setActiveTab('details')
      setFormError(null)
      setPathError(null)
      setAvailableFields([])
    }
  }, [open, permission])

  // Load field suggestions when node types change
  useEffect(() => {
    async function loadFieldSuggestions() {
      if (!repo || !branch || draft.node_types.length === 0) {
        setAvailableFields([])
        return
      }

      try {
        const allFields = new Set<string>()
        await Promise.all(
          draft.node_types.map(async (nodeType) => {
            try {
              const resolved = await nodeTypesApi.getResolved(repo, branch, nodeType)
              resolved.resolved_properties.forEach((prop) => {
                if (prop.name) allFields.add(prop.name)
              })
            } catch {
              // Ignore errors for individual node types
            }
          })
        )
        setAvailableFields(Array.from(allFields).sort())
      } catch (err) {
        console.error('Failed to load field suggestions:', err)
        setAvailableFields([])
      }
    }

    loadFieldSuggestions()
  }, [repo, branch, draft.node_types])

  // Validate path pattern on change
  function handlePathChange(newPath: string) {
    updateDraft('path', newPath)
    // Only validate if user has entered something
    if (newPath.trim()) {
      setPathError(validatePathPattern(newPath.trim()))
    } else {
      setPathError(null)
    }
  }

  const invalidNodeTypes =
    draft.node_types.length && nodeTypeOptions.length > 0
      ? draft.node_types.filter((type) => !nodeTypeSet.has(type))
      : []

  function updateDraft<K extends keyof PermissionDraft>(key: K, value: PermissionDraft[K]) {
    setDraft((prev) => ({ ...prev, [key]: value }))
  }

  function toggleOperation(operation: string) {
    setDraft((prev) => {
      const exists = prev.operations.includes(operation)
      const operations = exists
        ? prev.operations.filter((op) => op !== operation)
        : [...prev.operations, operation]
      return { ...prev, operations }
    })
  }

  function handleSave() {
    const trimmedPath = draft.path.trim()
    if (!trimmedPath) {
      setFormError('Permissions need a target path.')
      setActiveTab('details')
      return
    }
    // Validate path pattern
    const pathValidationError = validatePathPattern(trimmedPath)
    if (pathValidationError) {
      setFormError(pathValidationError)
      setPathError(pathValidationError)
      setActiveTab('details')
      return
    }
    if (draft.operations.length === 0) {
      setFormError('Select at least one operation.')
      setActiveTab('details')
      return
    }
    if (invalidNodeTypes.length > 0) {
      setFormError(`Unknown node types: ${invalidNodeTypes.join(', ')}`)
      setActiveTab('details')
      return
    }

    setFormError(null)
    onSave(normalizePermission({ ...draft, path: trimmedPath }))
  }

  if (!open) return null

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="glass-dark max-h-[90vh] w-full max-w-4xl animate-slide-in overflow-hidden rounded-2xl p-6 shadow-2xl">
        <div className="mb-4 flex items-start justify-between gap-4">
          <div>
            <h2 className="text-2xl font-semibold text-white">{title}</h2>
            <p className="text-sm text-zinc-400">
              Configure path, operations, and conditions for this permission.
            </p>
          </div>
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg p-2 text-zinc-400 transition-colors hover:bg-white/10"
            aria-label="Close permission editor"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="mb-4 flex gap-2 rounded-lg bg-white/5 p-1 text-sm text-zinc-300">
          <button
            type="button"
            onClick={() => setActiveTab('details')}
            className={`flex-1 rounded-md px-3 py-2 transition-colors ${
              activeTab === 'details' ? 'bg-primary-500/25 text-white' : 'hover:bg-white/10'
            }`}
          >
            Details
          </button>
          <button
            type="button"
            onClick={() => setActiveTab('conditions')}
            className={`flex-1 rounded-md px-3 py-2 transition-colors ${
              activeTab === 'conditions' ? 'bg-primary-500/25 text-white' : 'hover:bg-white/10'
            }`}
          >
            Conditions {draft.condition && '(active)'}
          </button>
        </div>

        {formError && (
          <div className="mb-4 rounded-lg border border-red-500/30 bg-red-500/15 px-4 py-3 text-sm text-red-200">
            {formError}
          </div>
        )}

        <div className="-mr-2 flex max-h-[60vh] flex-col gap-6 overflow-y-auto pr-2">
          {activeTab === 'details' ? (
            <div className="space-y-6">
              {/* Scope: Workspace and Branch Pattern */}
              <div className="grid gap-4 md:grid-cols-2">
                <div>
                  <label className="mb-2 block text-sm font-medium text-zinc-300">
                    Workspace
                  </label>
                  <input
                    type="text"
                    value={draft.workspace}
                    onChange={(e) => updateDraft('workspace', e.target.value)}
                    placeholder="* (all workspaces)"
                    className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
                  />
                  <p className="mt-1 text-xs text-zinc-500">e.g., "content", "marketing", or "*" for all</p>
                </div>
                <div>
                  <label className="mb-2 block text-sm font-medium text-zinc-300">
                    Branch Pattern
                  </label>
                  <input
                    type="text"
                    value={draft.branch_pattern}
                    onChange={(e) => updateDraft('branch_pattern', e.target.value)}
                    placeholder="* (all branches)"
                    className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
                  />
                  <p className="mt-1 text-xs text-zinc-500">e.g., "main", "features/*", or "*" for all</p>
                </div>
              </div>

              <div>
                <label className="mb-2 block text-sm font-medium text-zinc-300">
                  Path Pattern <span className="text-red-400">*</span>
                </label>
                <input
                  type="text"
                  value={draft.path}
                  onChange={(e) => handlePathChange(e.target.value)}
                  placeholder="/articles/** or /users/*/profile"
                  className={`w-full rounded-lg border bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:outline-none ${
                    pathError
                      ? 'border-red-500/50 focus:border-red-500'
                      : 'border-white/10 focus:border-primary-500'
                  }`}
                />
                {pathError ? (
                  <p className="mt-1 text-xs text-red-400">{pathError}</p>
                ) : (
                  <p className="mt-1 text-xs text-zinc-500">
                    Glob patterns: <code className="text-primary-300">**</code> matches recursively,{' '}
                    <code className="text-primary-300">*</code> matches single segment.
                    Examples: <code className="text-zinc-400">/articles/**</code>,{' '}
                    <code className="text-zinc-400">/users/*/profile</code>,{' '}
                    <code className="text-zinc-400">/00**</code>
                  </p>
                )}
              </div>

              <TagSelector
                label="Node Types"
                value={draft.node_types}
                onChange={(node_types) => updateDraft('node_types', node_types)}
                placeholder="Add node type..."
                suggestions={nodeTypeOptions}
                allowCustom={false}
                invalidValues={invalidNodeTypes}
                helperText={
                  nodeTypeOptions.length > 0
                    ? 'Pick from node types registered in this repository.'
                    : 'Node type suggestions unavailable.'
                }
                error={
                  invalidNodeTypes.length > 0
                    ? `Unknown node types: ${invalidNodeTypes.join(', ')}`
                    : undefined
                }
              />

              <div>
                <label className="mb-2 block text-sm font-medium text-zinc-300">
                  Operations <span className="text-red-400">*</span>
                </label>
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                  {OPERATIONS.map((op) => (
                    <label
                      key={op}
                      className={`flex items-center gap-2 rounded-lg border border-white/10 bg-white/5 px-3 py-2 transition-colors hover:bg-white/10 ${
                        draft.operations.includes(op) ? 'border-primary-500/50 bg-primary-500/15' : ''
                      }`}
                    >
                      <input
                        type="checkbox"
                        checked={draft.operations.includes(op)}
                        onChange={() => toggleOperation(op)}
                        className="h-4 w-4 rounded border-white/20 bg-white/10 text-primary-500 focus:ring-primary-500"
                      />
                      <span className="text-sm text-zinc-300">{formatOperationLabel(op)}</span>
                    </label>
                  ))}
                </div>
              </div>

              <div className="grid gap-4 md:grid-cols-2">
                <TagSelector
                  label="Allow Only These Fields (whitelist)"
                  value={draft.fields}
                  onChange={(fields) => updateDraft('fields', fields)}
                  placeholder="Add field name..."
                  suggestions={availableFields}
                  helperText={
                    availableFields.length > 0
                      ? 'Top-level property names only. If set, ONLY these fields will be visible. Takes precedence over blacklist.'
                      : 'Top-level property names only (e.g., title, content). Select node types to see field suggestions. If set, ONLY these fields will be visible.'
                  }
                />
                <TagSelector
                  label="Deny These Fields (blacklist)"
                  value={draft.except_fields}
                  onChange={(except_fields) => updateDraft('except_fields', except_fields)}
                  placeholder="Add field name..."
                  suggestions={availableFields}
                  helperText={
                    availableFields.length > 0
                      ? 'Top-level property names only. These fields will be hidden. Ignored if whitelist is set.'
                      : 'Top-level property names only. These fields will be hidden. Ignored if whitelist is set.'
                  }
                />
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              <div>
                <h3 className="text-base font-semibold text-white">ABAC Condition</h3>
                <p className="text-sm text-zinc-400">
                  Define when this permission applies using REL expressions.
                </p>
              </div>

              <ConditionBuilder
                condition={draft.condition}
                onChange={(condition) => updateDraft('condition', condition)}
                fieldPrefix="resource."
                placeholder="resource.author == auth.user_id && resource.status == 'published'"
              />

              <div className="rounded-lg border border-white/10 bg-white/5 p-4 text-xs text-zinc-400 space-y-2">
                <p className="font-medium text-zinc-300">Available variables:</p>
                <ul className="list-disc list-inside space-y-1 pl-2">
                  <li><code className="text-primary-300">resource.*</code> - Access node properties (e.g., resource.author, resource.status)</li>
                  <li><code className="text-primary-300">auth.user_id</code> - Current authenticated user ID</li>
                  <li><code className="text-primary-300">auth.email</code> - Current authenticated user email</li>
                  <li><code className="text-primary-300">auth.roles</code> - User's assigned roles</li>
                  <li><code className="text-primary-300">auth.groups</code> - User's group memberships</li>
                </ul>
              </div>
            </div>
          )}
        </div>

        <div className="mt-6 flex flex-wrap justify-between gap-3">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg bg-white/10 px-5 py-2 text-sm text-zinc-200 transition-colors hover:bg-white/20"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSave}
            className="flex items-center gap-2 rounded-lg bg-primary-500 px-6 py-2 text-sm text-white transition-colors hover:bg-primary-600"
          >
            <Layers className="h-4 w-4" />
            Save Permission
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}

export default function PermissionEditor({
  permissions,
  onChange,
  nodeTypeOptions = [],
  repo,
  branch,
}: PermissionEditorProps) {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)

  function handleSave(permission: Permission) {
    if (editingIndex === null) {
      onChange([...permissions, permission])
    } else {
      const next = [...permissions]
      next[editingIndex] = permission
      onChange(next)
    }
    setDialogOpen(false)
    setEditingIndex(null)
  }

  function openDialog(index: number | null) {
    setEditingIndex(index)
    setDialogOpen(true)
  }

  function removePermission(index: number) {
    onChange(permissions.filter((_, i) => i !== index))
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <label className="block text-sm font-medium text-zinc-300">Permissions</label>
        <button
          type="button"
          onClick={() => openDialog(null)}
          className="flex items-center gap-2 rounded-lg bg-primary-500 px-3 py-2 text-sm text-white transition-colors hover:bg-primary-600"
        >
          <Plus className="h-4 w-4" />
          Add Permission
        </button>
      </div>

      {permissions.length === 0 ? (
        <div className="rounded-xl border border-white/10 bg-white/5 py-8 text-center text-zinc-500">
          No permissions yet. Click "Add Permission" to get started.
        </div>
      ) : (
        <div className="space-y-4">
          {permissions.map((permission, index) => {
            const operationsLabel =
              permission.operations.length > 0
                ? permission.operations
                    .map((operation) => formatOperationLabel(operation))
                    .join(', ')
                : 'No operations'

            return (
              <div
                key={`permission-${index}`}
                className="rounded-xl border border-white/10 bg-white/5 p-4 shadow-sm"
              >
                <div className="flex flex-col gap-4 sm:flex-row sm:items-center">
                  <div className="flex-1 space-y-2">
                    <div className="flex flex-wrap items-center gap-2 text-xs font-medium uppercase tracking-wide text-zinc-500">
                      <span>Permission {index + 1}</span>
                      {permission.workspace && (
                        <span className="rounded-full border border-blue-500/30 bg-blue-500/10 px-2 py-1 text-[10px] normal-case text-blue-300">
                          ws: {permission.workspace}
                        </span>
                      )}
                      {permission.branch_pattern && (
                        <span className="rounded-full border border-green-500/30 bg-green-500/10 px-2 py-1 text-[10px] normal-case text-green-300">
                          branch: {permission.branch_pattern}
                        </span>
                      )}
                      {permission.node_types && permission.node_types.length > 0 && (
                        <span className="rounded-full border border-white/10 bg-white/10 px-2 py-1 text-[10px] text-zinc-300">
                          {permission.node_types.length} node type
                          {permission.node_types.length > 1 ? 's' : ''}
                        </span>
                      )}
                    </div>
                    <div className="text-lg font-semibold text-white">{permission.path}</div>
                    <div className="text-sm text-zinc-400">{operationsLabel}</div>
                  </div>
                  <div className="flex items-center gap-2 self-start sm:self-auto">
                    <button
                      type="button"
                      onClick={() => openDialog(index)}
                      className="flex items-center gap-2 rounded-lg border border-white/10 px-3 py-2 text-sm text-zinc-200 transition-colors hover:bg-white/10"
                    >
                      <Edit3 className="h-4 w-4" />
                      Edit
                    </button>
                    <button
                      type="button"
                      onClick={() => removePermission(index)}
                      className="rounded-lg border border-red-500/40 p-2 text-red-300 transition-colors hover:bg-red-500/20"
                      aria-label={`Remove permission ${index + 1}`}
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                </div>

                <div className="mt-3 flex flex-wrap gap-3 text-xs text-zinc-300">
                  {permission.condition ? (
                    <span className="rounded-full border border-primary-500/30 bg-primary-500/10 px-2 py-1 text-primary-300" title={permission.condition}>
                      Condition: {permission.condition.length > 40 ? `${permission.condition.slice(0, 40)}...` : permission.condition}
                    </span>
                  ) : (
                    <span className="rounded-full border border-white/10 bg-white/10 px-2 py-1">
                      No condition
                    </span>
                  )}
                  {permission.fields && permission.fields.length > 0 && (
                    <span className="rounded-full border border-white/10 bg-white/10 px-2 py-1">
                      {permission.fields.length} allowed field{permission.fields.length > 1 ? 's' : ''}
                    </span>
                  )}
                  {permission.except_fields && permission.except_fields.length > 0 && (
                    <span className="rounded-full border border-white/10 bg-white/10 px-2 py-1">
                      {permission.except_fields.length} blocked field
                      {permission.except_fields.length > 1 ? 's' : ''}
                    </span>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      )}

      {dialogOpen && (
        <PermissionDialog
          open={dialogOpen}
          title={editingIndex === null ? 'Add Permission' : `Edit Permission ${editingIndex + 1}`}
          permission={editingIndex === null ? undefined : permissions[editingIndex]}
          onCancel={() => {
            setDialogOpen(false)
            setEditingIndex(null)
          }}
          onSave={handleSave}
          nodeTypeOptions={nodeTypeOptions}
          repo={repo}
          branch={branch}
        />
      )}
    </div>
  )
}
