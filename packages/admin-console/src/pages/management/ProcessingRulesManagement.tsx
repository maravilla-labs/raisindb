import { useState, useEffect, useCallback } from 'react'
import {
  Plus,
  GripVertical,
  Edit,
  Trash2,
  FileType,
  FolderTree,
  FileCode,
  Layers,
  TestTube,
  CheckCircle,
  Loader2,
  ChevronDown,
  ChevronRight,
  Save,
  X,
  AlertCircle,
  Sparkles,
} from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import { ToastContainer, useToast } from '../../components/Toast'
import {
  processingRulesApi,
  ProcessingRule,
  RuleMatcher,
  ProcessingSettings,
  CreateRuleRequest,
  UpdateRuleRequest,
  TestRuleMatchRequest,
} from '../../api/processing-rules'
import { aiApi, LocalCaptionModel } from '../../api/ai'
import { ApiError } from '../../api/client'

interface ProcessingRulesManagementProps {
  repo: string
}

// Helper to format matcher for display
function formatMatcher(matcher: RuleMatcher): string {
  switch (matcher.type) {
    case 'all':
      return 'All content'
    case 'node_type':
      return `Type: ${matcher.node_type}`
    case 'path':
      return `Path: ${matcher.pattern}`
    case 'mime_type':
      return `MIME: ${matcher.mime_type}`
    case 'workspace':
      return `Workspace: ${matcher.workspace}`
    case 'property':
      return `Property: ${matcher.name}=${matcher.value}`
    case 'combined':
      return `Combined (${matcher.matchers.length} conditions)`
    default:
      return 'Unknown'
  }
}

function formatPdfStrategy(strategy: ProcessingSettings['pdf_strategy']): string {
  switch (strategy) {
    case 'auto':
      return 'Auto'
    case 'native_only':
      return 'Native Only'
    case 'ocr_only':
      return 'OCR Only'
    case 'force_ocr':
      return 'Force OCR'
    default:
      return 'Unknown'
  }
}

// Get icon for matcher type
function getMatcherIcon(matcher: RuleMatcher) {
  switch (matcher.type) {
    case 'node_type':
      return <FileType className="w-4 h-4 text-purple-400" />
    case 'path':
      return <FolderTree className="w-4 h-4 text-blue-400" />
    case 'mime_type':
      return <FileCode className="w-4 h-4 text-green-400" />
    case 'workspace':
      return <Layers className="w-4 h-4 text-yellow-400" />
    default:
      return <Sparkles className="w-4 h-4 text-gray-400" />
  }
}

export default function ProcessingRulesManagement({ repo }: ProcessingRulesManagementProps) {
  const toast = useToast()

  // State
  const [rules, setRules] = useState<ProcessingRule[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [editingRule, setEditingRule] = useState<ProcessingRule | null>(null)
  const [isCreating, setIsCreating] = useState(false)
  const [showTestPanel, setShowTestPanel] = useState(false)
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null)

  // Test panel state
  const [testInput, setTestInput] = useState<TestRuleMatchRequest>({
    path: '',
    node_type: '',
    mime_type: '',
    workspace: '',
  })
  const [testResult, setTestResult] = useState<{
    matched: boolean
    matched_rule?: ProcessingRule
    rules_evaluated: number
  } | null>(null)
  const [testing, setTesting] = useState(false)

  // Load rules
  const loadRules = useCallback(async () => {
    try {
      setLoading(true)
      const response = await processingRulesApi.listRules(repo)
      setRules(response.rules.sort((a, b) => a.order - b.order))
    } catch (error) {
      console.error('Failed to load rules:', error)
      // Note: toast is intentionally not in deps to prevent infinite loop
      toast.error('Failed to load rules', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setLoading(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo])

  useEffect(() => {
    loadRules()
  }, [loadRules])

  // Handle drag and drop
  const handleDragStart = (index: number) => {
    setDraggedIndex(index)
  }

  const handleDragOver = (e: React.DragEvent, index: number) => {
    e.preventDefault()
    if (draggedIndex === null || draggedIndex === index) return

    const newRules = [...rules]
    const [draggedRule] = newRules.splice(draggedIndex, 1)
    newRules.splice(index, 0, draggedRule)
    setRules(newRules)
    setDraggedIndex(index)
  }

  const handleDragEnd = async () => {
    if (draggedIndex !== null) {
      try {
        setSaving(true)
        await processingRulesApi.reorderRules(repo, rules.map((r) => r.id))
        toast.success('Rules reordered', 'The rule order has been updated')
      } catch (error) {
        console.error('Failed to reorder rules:', error)
        toast.error('Failed to reorder', error instanceof ApiError ? error.message : 'Unknown error')
        await loadRules() // Reload to reset order
      } finally {
        setSaving(false)
      }
    }
    setDraggedIndex(null)
  }

  // Handle create rule
  const handleCreateRule = async (request: CreateRuleRequest) => {
    try {
      setSaving(true)
      await processingRulesApi.createRule(repo, request)
      toast.success('Rule created', `Rule "${request.name}" has been created`)
      setIsCreating(false)
      await loadRules()
    } catch (error) {
      console.error('Failed to create rule:', error)
      toast.error('Failed to create rule', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setSaving(false)
    }
  }

  // Handle update rule
  const handleUpdateRule = async (ruleId: string, request: UpdateRuleRequest) => {
    try {
      setSaving(true)
      await processingRulesApi.updateRule(repo, ruleId, request)
      toast.success('Rule updated', 'The rule has been updated')
      setEditingRule(null)
      await loadRules()
    } catch (error) {
      console.error('Failed to update rule:', error)
      toast.error('Failed to update rule', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setSaving(false)
    }
  }

  // Handle delete rule
  const handleDeleteRule = async (rule: ProcessingRule) => {
    if (!confirm(`Delete rule "${rule.name}"? This cannot be undone.`)) return

    try {
      setSaving(true)
      await processingRulesApi.deleteRule(repo, rule.id)
      toast.success('Rule deleted', `Rule "${rule.name}" has been deleted`)
      await loadRules()
    } catch (error) {
      console.error('Failed to delete rule:', error)
      toast.error('Failed to delete rule', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setSaving(false)
    }
  }

  // Handle toggle rule enabled
  const handleToggleEnabled = async (rule: ProcessingRule) => {
    try {
      await processingRulesApi.updateRule(repo, rule.id, { enabled: !rule.enabled })
      setRules(
        rules.map((r) =>
          r.id === rule.id ? { ...r, enabled: !r.enabled } : r
        )
      )
    } catch (error) {
      console.error('Failed to toggle rule:', error)
      toast.error('Failed to update rule', error instanceof ApiError ? error.message : 'Unknown error')
    }
  }

  // Handle test rule matching
  const handleTestRules = async () => {
    try {
      setTesting(true)
      setTestResult(null)
      const result = await processingRulesApi.testRuleMatch(repo, testInput)
      setTestResult(result)
    } catch (error) {
      console.error('Failed to test rules:', error)
      toast.error('Failed to test', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setTesting(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="w-8 h-8 text-purple-400 animate-spin" />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <ToastContainer toasts={toast.toasts} onClose={toast.closeToast} />

      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-bold text-white flex items-center gap-2">
            <Sparkles className="w-6 h-6 text-purple-400" />
            AI Processing Rules
          </h2>
          <p className="text-gray-400 mt-1">
            Configure how content is processed for embeddings, captioning, and OCR.
            Rules are evaluated in order (first match wins).
          </p>
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={() => setShowTestPanel(!showTestPanel)}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-all flex items-center gap-2"
          >
            <TestTube className="w-4 h-4" />
            {showTestPanel ? 'Hide Test' : 'Test Rules'}
          </button>
          <button
            onClick={() => setIsCreating(true)}
            disabled={saving}
            className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 text-white rounded-lg transition-all flex items-center gap-2"
          >
            <Plus className="w-4 h-4" />
            Add Rule
          </button>
        </div>
      </div>

      {/* Test Panel */}
      {showTestPanel && (
        <GlassCard>
          <h3 className="text-lg font-medium text-white mb-4 flex items-center gap-2">
            <TestTube className="w-5 h-5 text-purple-400" />
            Test Rule Matching
          </h3>
          <div className="grid grid-cols-2 gap-4 mb-4">
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-1">
                Path
              </label>
              <input
                type="text"
                value={testInput.path || ''}
                onChange={(e) => setTestInput({ ...testInput, path: e.target.value })}
                placeholder="/documents/report.pdf"
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-1">
                Node Type
              </label>
              <input
                type="text"
                value={testInput.node_type || ''}
                onChange={(e) => setTestInput({ ...testInput, node_type: e.target.value })}
                placeholder="raisin:Asset"
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-1">
                MIME Type
              </label>
              <input
                type="text"
                value={testInput.mime_type || ''}
                onChange={(e) => setTestInput({ ...testInput, mime_type: e.target.value })}
                placeholder="application/pdf"
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-1">
                Workspace
              </label>
              <input
                type="text"
                value={testInput.workspace || ''}
                onChange={(e) => setTestInput({ ...testInput, workspace: e.target.value })}
                placeholder="content"
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
              />
            </div>
          </div>
          <div className="flex items-center gap-4">
            <button
              onClick={handleTestRules}
              disabled={testing}
              className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 text-white rounded-lg transition-all flex items-center gap-2"
            >
              {testing ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <TestTube className="w-4 h-4" />
              )}
              Test Match
            </button>
            {testResult && (
              <div
                className={`flex items-center gap-2 px-4 py-2 rounded-lg ${
                  testResult.matched
                    ? 'bg-green-500/10 border border-green-500/30'
                    : 'bg-yellow-500/10 border border-yellow-500/30'
                }`}
              >
                {testResult.matched ? (
                  <>
                    <CheckCircle className="w-4 h-4 text-green-400" />
                    <span className="text-green-300 text-sm">
                      Matched: {testResult.matched_rule?.name}
                    </span>
                  </>
                ) : (
                  <>
                    <AlertCircle className="w-4 h-4 text-yellow-400" />
                    <span className="text-yellow-300 text-sm">
                      No rule matched ({testResult.rules_evaluated} evaluated)
                    </span>
                  </>
                )}
              </div>
            )}
          </div>
        </GlassCard>
      )}

      {/* Create/Edit Rule Form */}
      {(isCreating || editingRule) && (
        <RuleEditor
          rule={editingRule}
          onSave={(request) => {
            if (editingRule) {
              handleUpdateRule(editingRule.id, request)
            } else {
              handleCreateRule(request as CreateRuleRequest)
            }
          }}
          onCancel={() => {
            setIsCreating(false)
            setEditingRule(null)
          }}
          saving={saving}
        />
      )}

      {/* Rules List */}
      <div className="space-y-2">
        {rules.length === 0 ? (
          <GlassCard>
            <div className="text-center py-8">
              <Sparkles className="w-12 h-12 text-gray-500 mx-auto mb-4" />
              <p className="text-gray-400 mb-4">No processing rules configured</p>
              <button
                onClick={() => setIsCreating(true)}
                className="px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-all inline-flex items-center gap-2"
              >
                <Plus className="w-4 h-4" />
                Create First Rule
              </button>
            </div>
          </GlassCard>
        ) : (
          rules.map((rule, index) => (
            <div
              key={rule.id}
              draggable
              onDragStart={() => handleDragStart(index)}
              onDragOver={(e) => handleDragOver(e, index)}
              onDragEnd={handleDragEnd}
              className={`bg-white/5 border border-white/10 rounded-lg p-4 transition-all ${
                draggedIndex === index ? 'opacity-50 scale-[0.98]' : ''
              }`}
            >
              <div className="flex items-center gap-4">
                {/* Drag Handle */}
                <div className="cursor-grab active:cursor-grabbing text-gray-500 hover:text-gray-300">
                  <GripVertical className="w-5 h-5" />
                </div>

                {/* Priority Badge */}
                <div className="w-8 h-8 rounded-full bg-purple-500/20 border border-purple-500/30 flex items-center justify-center text-sm font-bold text-purple-300">
                  {index + 1}
                </div>

                {/* Rule Info */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="text-white font-medium">{rule.name}</h3>
                    {!rule.enabled && (
                      <span className="px-2 py-0.5 bg-gray-500/20 border border-gray-500/30 text-gray-400 text-xs rounded">
                        Disabled
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-2 mt-1">
                    {getMatcherIcon(rule.matcher)}
                    <span className="text-gray-400 text-sm">
                      {formatMatcher(rule.matcher)}
                    </span>
                  </div>
                </div>

                {/* Settings Summary */}
                <div className="flex items-center gap-2">
                  {rule.settings.generate_image_embedding && (
                    <span className="px-2 py-0.5 bg-blue-500/20 border border-blue-500/30 text-blue-300 text-xs rounded">
                      Image Embed
                    </span>
                  )}
                  {rule.settings.generate_image_caption && (
                    <span className="px-2 py-0.5 bg-green-500/20 border border-green-500/30 text-green-300 text-xs rounded" title={rule.settings.caption_model || 'Default model'}>
                      Caption{rule.settings.caption_model ? `: ${rule.settings.caption_model.split('/').pop()}` : ''}
                    </span>
                  )}
                  {rule.settings.generate_keywords && (
                    <span className="px-2 py-0.5 bg-teal-500/20 border border-teal-500/30 text-teal-300 text-xs rounded">
                      Keywords
                    </span>
                  )}
                  {rule.settings.pdf_strategy && (
                    <span className="px-2 py-0.5 bg-yellow-500/20 border border-yellow-500/30 text-yellow-300 text-xs rounded">
                      PDF: {formatPdfStrategy(rule.settings.pdf_strategy)}
                    </span>
                  )}
                  {rule.settings.chunking && (
                    <span className="px-2 py-0.5 bg-purple-500/20 border border-purple-500/30 text-purple-300 text-xs rounded">
                      Chunk: {rule.settings.chunking.chunk_size}
                    </span>
                  )}
                </div>

                {/* Actions */}
                <div className="flex items-center gap-2">
                  <label className="relative inline-flex items-center cursor-pointer">
                    <input
                      type="checkbox"
                      checked={rule.enabled}
                      onChange={() => handleToggleEnabled(rule)}
                      className="sr-only peer"
                    />
                    <div className="w-9 h-5 bg-white/10 peer-checked:bg-purple-500 rounded-full transition-colors"></div>
                    <div className="absolute left-0.5 top-0.5 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-4"></div>
                  </label>
                  <button
                    onClick={() => setEditingRule(rule)}
                    className="p-2 text-gray-400 hover:text-white hover:bg-white/10 rounded-lg transition-colors"
                    title="Edit rule"
                  >
                    <Edit className="w-4 h-4" />
                  </button>
                  <button
                    onClick={() => handleDeleteRule(rule)}
                    className="p-2 text-gray-400 hover:text-red-400 hover:bg-red-500/10 rounded-lg transition-colors"
                    title="Delete rule"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  )
}

// =============================================================================
// Rule Editor Component
// =============================================================================

interface RuleEditorProps {
  rule: ProcessingRule | null
  onSave: (request: CreateRuleRequest | UpdateRuleRequest) => void
  onCancel: () => void
  saving: boolean
}

function RuleEditor({ rule, onSave, onCancel, saving }: RuleEditorProps) {
  const [name, setName] = useState(rule?.name || '')
  const [matcherType, setMatcherType] = useState<RuleMatcher['type']>(rule?.matcher?.type || 'all')
  const [matcherValue, setMatcherValue] = useState('')
  const [matcherProperty, setMatcherProperty] = useState({ name: '', value: '' })
  const [settings, setSettings] = useState<ProcessingSettings>(rule?.settings || {})
  const [expanded, setExpanded] = useState(true)

  // Caption model state
  const [captionModels, setCaptionModels] = useState<LocalCaptionModel[]>([])
  const [defaultCaptionModel, setDefaultCaptionModel] = useState('')
  const [loadingModels, setLoadingModels] = useState(true)

  // Load caption models on mount
  useEffect(() => {
    const loadModels = async () => {
      try {
        const response = await aiApi.listLocalCaptionModels()
        setCaptionModels(response.models.filter(m => m.supported))
        setDefaultCaptionModel(response.default_model)
      } catch (error) {
        console.error('Failed to load caption models:', error)
        // Fallback to hardcoded defaults if API fails
        setCaptionModels([
          { id: 'Salesforce/blip-image-captioning-large', name: 'BLIP Large', size_mb: 1880, supported: true, description: 'High quality captioning' },
          { id: 'lmz/candle-blip', name: 'BLIP Large (Quantized)', size_mb: 271, supported: true, description: 'Faster CPU inference' },
        ])
        setDefaultCaptionModel('Salesforce/blip-image-captioning-large')
      } finally {
        setLoadingModels(false)
      }
    }
    loadModels()
  }, [])

  // Initialize matcher value from rule
  useEffect(() => {
    if (rule?.matcher) {
      switch (rule.matcher.type) {
        case 'node_type':
          setMatcherValue(rule.matcher.node_type)
          break
        case 'path':
          setMatcherValue(rule.matcher.pattern)
          break
        case 'mime_type':
          setMatcherValue(rule.matcher.mime_type)
          break
        case 'workspace':
          setMatcherValue(rule.matcher.workspace)
          break
        case 'property':
          setMatcherProperty({ name: rule.matcher.name, value: rule.matcher.value })
          break
      }
    }
  }, [rule])

  const buildMatcher = (): RuleMatcher => {
    switch (matcherType) {
      case 'all':
        return { type: 'all' }
      case 'node_type':
        return { type: 'node_type', node_type: matcherValue }
      case 'path':
        return { type: 'path', pattern: matcherValue }
      case 'mime_type':
        return { type: 'mime_type', mime_type: matcherValue }
      case 'workspace':
        return { type: 'workspace', workspace: matcherValue }
      case 'property':
        return { type: 'property', name: matcherProperty.name, value: matcherProperty.value }
      default:
        return { type: 'all' }
    }
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    onSave({
      name,
      matcher: buildMatcher(),
      settings,
    })
  }

  return (
    <GlassCard>
      <form onSubmit={handleSubmit} className="space-y-4">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-medium text-white">
            {rule ? 'Edit Rule' : 'Create Rule'}
          </h3>
          <button
            type="button"
            onClick={onCancel}
            className="p-1 text-gray-400 hover:text-white hover:bg-white/10 rounded transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Rule Name */}
        <div>
          <label className="block text-sm font-medium text-gray-300 mb-1">
            Rule Name
          </label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="PDF Documents"
            required
            className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
          />
        </div>

        {/* Matcher Type */}
        <div>
          <label className="block text-sm font-medium text-gray-300 mb-1">
            Match Condition
          </label>
          <select
            value={matcherType}
            onChange={(e) => setMatcherType(e.target.value as RuleMatcher['type'])}
            className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
          >
            <option value="all" className="bg-gray-900">All content (catch-all)</option>
            <option value="node_type" className="bg-gray-900">Node Type</option>
            <option value="path" className="bg-gray-900">Path Pattern (glob)</option>
            <option value="mime_type" className="bg-gray-900">MIME Type</option>
            <option value="workspace" className="bg-gray-900">Workspace</option>
            <option value="property" className="bg-gray-900">Property Value</option>
          </select>
        </div>

        {/* Matcher Value */}
        {matcherType !== 'all' && matcherType !== 'property' && (
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-1">
              {matcherType === 'node_type' && 'Node Type (e.g., raisin:Asset)'}
              {matcherType === 'path' && 'Path Pattern (e.g., /documents/**)'}
              {matcherType === 'mime_type' && 'MIME Type (e.g., application/pdf)'}
              {matcherType === 'workspace' && 'Workspace Name'}
            </label>
            <input
              type="text"
              value={matcherValue}
              onChange={(e) => setMatcherValue(e.target.value)}
              placeholder={
                matcherType === 'node_type'
                  ? 'raisin:Asset'
                  : matcherType === 'path'
                  ? '/documents/**'
                  : matcherType === 'mime_type'
                  ? 'application/pdf'
                  : 'content'
              }
              required
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
            />
          </div>
        )}

        {/* Property Matcher */}
        {matcherType === 'property' && (
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-1">
                Property Name
              </label>
              <input
                type="text"
                value={matcherProperty.name}
                onChange={(e) =>
                  setMatcherProperty({ ...matcherProperty, name: e.target.value })
                }
                placeholder="category"
                required
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-1">
                Property Value
              </label>
              <input
                type="text"
                value={matcherProperty.value}
                onChange={(e) =>
                  setMatcherProperty({ ...matcherProperty, value: e.target.value })
                }
                placeholder="images"
                required
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
              />
            </div>
          </div>
        )}

        {/* Processing Settings */}
        <div className="border-t border-white/10 pt-4">
          <button
            type="button"
            onClick={() => setExpanded(!expanded)}
            className="flex items-center gap-2 text-white font-medium mb-3"
          >
            {expanded ? (
              <ChevronDown className="w-4 h-4" />
            ) : (
              <ChevronRight className="w-4 h-4" />
            )}
            Processing Settings
          </button>

          {expanded && (
            <div className="space-y-4 pl-6">
              {/* Image Embedding */}
              <label className="flex items-center gap-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={settings.generate_image_embedding || false}
                  onChange={(e) =>
                    setSettings({
                      ...settings,
                      generate_image_embedding: e.target.checked,
                    })
                  }
                  className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500"
                />
                <div>
                  <span className="text-white font-medium">Generate Image Embeddings</span>
                  <p className="text-gray-400 text-xs">Use CLIP to generate embeddings for images</p>
                </div>
              </label>

              {/* Image Captioning */}
              <div className="space-y-2">
                <label className="flex items-center gap-3 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={settings.generate_image_caption || false}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        generate_image_caption: e.target.checked,
                        // Reset model when unchecking
                        caption_model: e.target.checked ? settings.caption_model : undefined,
                      })
                    }
                    className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500"
                  />
                  <div>
                    <span className="text-white font-medium">Generate Image Captions</span>
                    <p className="text-gray-400 text-xs">Auto-generate alt text for images</p>
                  </div>
                </label>

                {/* Caption Model Selector (shown when caption generation is enabled) */}
                {settings.generate_image_caption && (
                  <div className="ml-7 mt-2">
                    <label className="block text-sm text-gray-300 mb-1">
                      Caption Model <span className="text-gray-500">(optional override)</span>
                    </label>
                    <select
                      value={settings.caption_model || ''}
                      onChange={(e) =>
                        setSettings({
                          ...settings,
                          caption_model: e.target.value || undefined,
                          // Clear custom prompts when switching to BLIP (which doesn't support them)
                          alt_text_prompt: e.target.value?.toLowerCase().includes('blip') ? undefined : settings.alt_text_prompt,
                          description_prompt: e.target.value?.toLowerCase().includes('blip') ? undefined : settings.description_prompt,
                        })
                      }
                      disabled={loadingModels}
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 disabled:opacity-50"
                    >
                      <option value="" className="bg-gray-900">
                        Use tenant default ({defaultCaptionModel.split('/').pop()})
                      </option>
                      {captionModels.map((model) => (
                        <option key={model.id} value={model.id} className="bg-gray-900">
                          {model.name} ({model.size_mb} MB)
                        </option>
                      ))}
                    </select>
                    <p className="text-gray-500 text-xs mt-1">
                      Override the tenant-level default caption model for this rule
                    </p>

                    {/* Custom Prompts & Keywords (Moondream only) */}
                    {(!settings.caption_model || settings.caption_model.toLowerCase().includes('moondream')) && (
                      <div className="mt-4 p-3 bg-white/5 rounded-lg border border-white/10">
                        <p className="text-sm text-purple-300 font-medium mb-3">
                          Custom Prompts <span className="text-gray-500 font-normal">(Moondream only)</span>
                        </p>

                        <div className="space-y-3">
                          <div>
                            <label className="block text-xs text-gray-400 mb-1">
                              Alt-Text Prompt
                            </label>
                            <input
                              type="text"
                              value={settings.alt_text_prompt || ''}
                              onChange={(e) =>
                                setSettings({
                                  ...settings,
                                  alt_text_prompt: e.target.value || undefined,
                                })
                              }
                              placeholder="Describe this image briefly in one sentence for accessibility."
                              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
                            />
                          </div>

                          <div>
                            <label className="block text-xs text-gray-400 mb-1">
                              Description Prompt
                            </label>
                            <input
                              type="text"
                              value={settings.description_prompt || ''}
                              onChange={(e) =>
                                setSettings({
                                  ...settings,
                                  description_prompt: e.target.value || undefined,
                                })
                              }
                              placeholder="Describe this image in detail."
                              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
                            />
                          </div>
                        </div>

                        <p className="text-gray-500 text-xs mt-2">
                          Leave empty to use default prompts. Custom prompts let you tailor the AI's output for your specific use case.
                        </p>

                        {/* Keywords Generation */}
                        <div className="mt-4 pt-3 border-t border-white/10">
                          <label className="flex items-center gap-3 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={settings.generate_keywords || false}
                              onChange={(e) =>
                                setSettings({
                                  ...settings,
                                  generate_keywords: e.target.checked,
                                  keywords_prompt: e.target.checked ? settings.keywords_prompt : undefined,
                                })
                              }
                              className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500"
                            />
                            <div>
                              <span className="text-white font-medium text-sm">Generate Keywords</span>
                              <p className="text-gray-400 text-xs">Extract descriptive keywords from images</p>
                            </div>
                          </label>

                          {settings.generate_keywords && (
                            <div className="mt-3 ml-7">
                              <label className="block text-xs text-gray-400 mb-1">
                                Keywords Prompt
                              </label>
                              <input
                                type="text"
                                value={settings.keywords_prompt || ''}
                                onChange={(e) =>
                                  setSettings({
                                    ...settings,
                                    keywords_prompt: e.target.value || undefined,
                                  })
                                }
                                placeholder="List 5-10 keywords that describe this image, separated by commas."
                                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 text-sm focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
                              />
                              <p className="text-gray-500 text-xs mt-1">
                                Keywords will be stored as an array on the node for search and filtering.
                              </p>
                            </div>
                          )}
                        </div>
                      </div>
                    )}
                  </div>
                )}
              </div>

              {/* PDF Strategy */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  PDF Processing Strategy
                </label>
                <select
                  value={settings.pdf_strategy || ''}
                  onChange={(e) =>
                    setSettings({
                      ...settings,
                      pdf_strategy: (e.target.value || undefined) as ProcessingSettings['pdf_strategy'],
                    })
                  }
                  className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20"
                >
                  <option value="" className="bg-gray-900">Default (inherit global)</option>
                  <option value="auto" className="bg-gray-900">Auto (try native, fallback OCR)</option>
                  <option value="native_only" className="bg-gray-900">Native Only (text extraction)</option>
                  <option value="ocr_only" className="bg-gray-900">OCR Only (image-based)</option>
                  <option value="force_ocr" className="bg-gray-900">Force OCR (user override)</option>
                </select>
              </div>

              {/* Chunking Override */}
              <div>
                <label className="flex items-center gap-3 cursor-pointer mb-2">
                  <input
                    type="checkbox"
                    checked={!!settings.chunking}
                    onChange={(e) =>
                      setSettings({
                        ...settings,
                        chunking: e.target.checked
                          ? { chunk_size: 256, overlap: { type: 'Tokens', value: 64 }, splitter: 'recursive' }
                          : undefined,
                      })
                    }
                    className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500"
                  />
                  <span className="text-white font-medium">Override Chunking Settings</span>
                </label>
                {settings.chunking && (
                  <div className="pl-7 space-y-3">
                    <div>
                      <label className="block text-sm text-gray-300 mb-1">
                        Chunk Size (tokens)
                      </label>
                      <input
                        type="number"
                        min="64"
                        max="1024"
                        step="64"
                        value={settings.chunking.chunk_size}
                        onChange={(e) =>
                          setSettings({
                            ...settings,
                            chunking: {
                              ...settings.chunking!,
                              chunk_size: parseInt(e.target.value) || 256,
                            },
                          })
                        }
                        className="w-32 px-3 py-1 bg-white/5 border border-white/10 rounded-lg text-white"
                      />
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>

        {/* Actions */}
        <div className="flex items-center justify-end gap-3 pt-4 border-t border-white/10">
          <button
            type="button"
            onClick={onCancel}
            disabled={saving}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-all"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={saving || !name}
            className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 disabled:text-gray-500 text-white rounded-lg transition-all flex items-center gap-2"
          >
            {saving ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Save className="w-4 h-4" />
            )}
            {rule ? 'Update Rule' : 'Create Rule'}
          </button>
        </div>
      </form>
    </GlassCard>
  )
}
