import { api, requestRaw } from './client'

// Types

export interface PackageSummary {
  name: string
  version: string
  title?: string
  description?: string
  author?: string
  installed: boolean
  category?: string
  keywords?: string[]
  icon?: string    // Lucide icon name (e.g., "sparkles") or URL
  color?: string   // Hex color (e.g., "#8B5CF6")
  /** Upload state: 'new' for first upload, 'updated' for re-uploads */
  upload_state?: 'new' | 'updated'
  /** Relative path to teaser background image (e.g., "static/teaser_background.png") */
  teaser_background_url?: string
  created_at?: string
  updated_at?: string
}

export interface PackageDetails {
  name: string
  version: string
  title?: string
  description?: string
  author?: string
  installed: boolean
  category?: string
  keywords?: string[]
  icon?: string    // Lucide icon name (e.g., "sparkles") or URL
  color?: string   // Hex color (e.g., "#8B5CF6")
  /** Upload state: 'new' for first upload, 'updated' for re-uploads */
  upload_state?: 'new' | 'updated'
  /** Relative path to teaser background image (e.g., "static/teaser_background.png") */
  teaser_background_url?: string
  dependencies?: PackageDependency[]
  provides?: PackageProvides
  changelog?: string
  readme?: string
  created_at?: string
  updated_at?: string
}

export interface PackageDependency {
  name: string
  version: string
  optional?: boolean
}

export interface PackageProvides {
  node_types?: string[]
  workspaces?: string[]
  functions?: string[]
  content?: string[]
}

export interface PackageFile {
  path: string
  name: string
  type: 'file' | 'directory'
  size?: number
  mime_type?: string
}

export interface PackageManifest {
  name: string
  version: string
  title?: string
  description?: string
  author?: string
  category?: string
  keywords?: string[]
  icon?: string
  dependencies?: PackageDependency[]
  provides?: PackageProvides
}

export interface ListPackagesParams {
  installed?: boolean
  category?: string
  keyword?: string
  limit?: number
  offset?: number
}

export interface UploadPackageResponse {
  package_name: string
  version: string
  node_id: string
  /** Job ID for tracking background processing (large uploads) */
  job_id?: string
  /** Status: 'processing' for large uploads, undefined for small uploads */
  status?: string
}

export interface InstallPackageResponse {
  package_name: string
  version: string
  installed: boolean
  installed_at?: string
  /** Job ID for tracking async installation progress */
  job_id?: string
}

/**
 * Install mode for handling conflicts during package installation
 * - 'skip': Only install to paths that don't exist (default) - preserve existing content
 * - 'overwrite': Delete and replace all existing content from the package
 * - 'sync': Update existing content (upsert), create new content, leave untouched content alone
 */
export type InstallMode = 'skip' | 'overwrite' | 'sync'

export interface InstallPackageOptions {
  /** Install mode for conflict handling */
  mode?: InstallMode
  /** Branch to install to (defaults to 'main') */
  branch?: string
}

// ============================================
// Dry Run Types
// ============================================

/** A single log entry from the dry run simulation */
export interface DryRunLogEntry {
  /** Log level: "info", "create", "update", "skip" */
  level: 'info' | 'create' | 'update' | 'skip'
  /** Category: "node_type", "workspace", "content", "binary", "archetype", "element_type", "package_asset" */
  category: string
  /** Path or name of the item */
  path: string
  /** Human-readable message */
  message: string
  /** Action that would be taken: "info", "create", "update", "skip" */
  action: 'info' | 'create' | 'update' | 'skip'
}

/** Counts of create/update/skip actions */
export interface DryRunActionCounts {
  create: number
  update: number
  skip: number
}

/** Summary of actions that would be taken */
export interface DryRunSummary {
  node_types: DryRunActionCounts
  archetypes: DryRunActionCounts
  element_types: DryRunActionCounts
  workspaces: DryRunActionCounts
  content_nodes: DryRunActionCounts
  binary_files: DryRunActionCounts
  package_assets: DryRunActionCounts
}

/** Response from dry run preview endpoint */
export interface DryRunResponse {
  package_name: string
  package_version: string
  mode: InstallMode
  logs: DryRunLogEntry[]
  summary: DryRunSummary
}

// API Functions

/**
 * List all packages in a repository
 */
export async function listPackages(
  repo: string,
  params?: ListPackagesParams
): Promise<PackageSummary[]> {
  const searchParams = new URLSearchParams()
  if (params?.installed !== undefined) searchParams.set('installed', String(params.installed))
  if (params?.category) searchParams.set('category', params.category)
  if (params?.keyword) searchParams.set('keyword', params.keyword)
  if (params?.limit) searchParams.set('limit', String(params.limit))
  if (params?.offset) searchParams.set('offset', String(params.offset))

  const query = searchParams.toString()
  const path = `/api/repos/${repo}/packages${query ? `?${query}` : ''}`
  return api.get<PackageSummary[]>(path)
}

/**
 * Get package details using unified node endpoint
 */
export async function getPackage(
  repo: string,
  name: string,
  branch = 'main'
): Promise<PackageDetails> {
  // Use unified node endpoint - packages are stored in 'packages' workspace
  const node = await api.get<{
    id: string
    name: string
    path: string
    node_type: string
    properties?: Record<string, unknown>
  }>(`/api/repository/${repo}/${branch}/head/packages/${encodeURIComponent(name)}`)

  // Map node properties to PackageDetails format
  const props = node.properties || {}
  return {
    name: node.name,
    version: props.version as string || '',
    title: props.title as string,
    description: props.description as string,
    author: props.author as string,
    installed: props.installed as boolean || false,
    category: props.category as string,
    keywords: props.keywords as string[],
    icon: props.icon as string,
    color: props.color as string,
    upload_state: props.upload_state as 'new' | 'updated',
    teaser_background_url: props.teaser_background_url as string,
    dependencies: props.dependencies as PackageDependency[],
    provides: props.provides as PackageProvides,
    changelog: props.changelog as string,
    readme: props.readme as string,
    created_at: props.created_at as string,
    updated_at: props.updated_at as string,
  }
}

/**
 * Upload a new package (.rap file)
 *
 * Uses the unified endpoint which creates a raisin:Package node.
 * A background job will process the manifest and update node properties.
 *
 * @param repo - Repository name
 * @param file - The .rap file to upload
 * @param targetPath - Optional target folder path (e.g., '/my-folder'). Defaults to root.
 * @param branch - Optional branch name. Defaults to 'main'.
 */
export async function uploadPackage(
  repo: string,
  file: File,
  targetPath?: string,
  branch = 'main'
): Promise<UploadPackageResponse> {
  const formData = new FormData()
  formData.append('file', file)

  // Use unified endpoint with nodeType parameter (camelCase per RepoQuery struct)
  // The filename becomes the node name (without .rap extension)
  const filename = file.name.replace(/\.rap$/, '')

  // Build the full path: targetPath + filename
  // e.g., targetPath='/my-folder' + filename='ai-tools' => '/my-folder/ai-tools'
  const normalizedPath = targetPath && targetPath !== '/'
    ? `${targetPath.replace(/\/$/, '')}/${encodeURIComponent(filename)}`
    : `/${encodeURIComponent(filename)}`

  // Remove leading slash for URL (endpoint expects path without leading slash)
  const urlPath = normalizedPath.replace(/^\//, '')

  const response = await api.post<{ storedKey: string; url: string; node_id?: string; job_id?: string }>(
    `/api/repository/${repo}/${branch}/head/packages/${urlPath}?nodeType=raisin:Package`,
    formData
  )

  // Map response to expected format
  return {
    package_name: filename,
    version: 'processing', // Will be updated by background job
    node_id: response.node_id || '',
    job_id: response.job_id,
  }
}

/**
 * Install a package using the dedicated packages endpoint
 *
 * @param repo - Repository name
 * @param name - Package name
 * @param options - Optional install options (mode, branch)
 *
 * Uses the unified endpoint:
 * POST /api/packages/{repo}/{branch}/head/{name}/raisin:install?mode={mode}
 */
export async function installPackage(
  repo: string,
  name: string,
  options?: InstallPackageOptions
): Promise<InstallPackageResponse> {
  const branch = options?.branch ?? 'main'
  const mode = options?.mode ?? 'skip'
  const query = mode !== 'skip' ? `?mode=${mode}` : ''

  return api.post<InstallPackageResponse>(
    `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:install${query}`
  )
}

/**
 * Dry run package installation preview
 *
 * Returns a preview of what would happen during installation without making any changes.
 * This is a read-only operation that returns synchronously.
 *
 * @param repo - Repository name
 * @param name - Package name
 * @param options - Optional preview options (mode, branch)
 */
export async function dryRunPackage(
  repo: string,
  name: string,
  options?: InstallPackageOptions
): Promise<DryRunResponse> {
  const branch = options?.branch ?? 'main'
  const mode = options?.mode ?? 'skip'
  const query = `?mode=${mode}`

  return api.get<DryRunResponse>(
    `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:dry-run${query}`
  )
}

/**
 * Uninstall a package
 */
export async function uninstallPackage(
  repo: string,
  name: string
): Promise<InstallPackageResponse> {
  return api.post<InstallPackageResponse>(
    `/api/repos/${repo}/packages/${name}/uninstall`
  )
}

/**
 * Browse package ZIP contents (branch-aware)
 * Uses dedicated /api/packages endpoint with hardcoded workspace:
 * /api/packages/{repo}/{branch}/head/{package_path}/raisin:browse/{zip_path}
 */
export async function browsePackageContents(
  repo: string,
  name: string,
  path = '',
  branch = 'main'
): Promise<PackageFile[]> {
  // Path is part of URL after raisin:browse command
  const zipPath = path ? `/${path}` : ''
  return api.get<PackageFile[]>(
    `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:browse${zipPath}`
  )
}

/**
 * Get a file from package ZIP (branch-aware)
 * Uses dedicated /api/packages endpoint with hardcoded workspace:
 * /api/packages/{repo}/{branch}/head/{package_path}/raisin:file/{zip_path}
 */
export async function getPackageFile(
  repo: string,
  name: string,
  path: string,
  branch = 'main'
): Promise<string> {
  const response = await requestRaw(
    `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:file/${path}`
  )

  return response.text()
}

/**
 * Delete a package
 */
export async function deletePackage(
  repo: string,
  name: string
): Promise<void> {
  return api.delete<void>(`/api/repos/${repo}/packages/${name}`)
}

// ============================================
// Sync & Export Types
// ============================================

export type SyncFileStatus =
  | 'synced'
  | 'modified'
  | 'local_only'
  | 'server_only'
  | 'conflict'

export interface SyncFileInfo {
  path: string
  status: SyncFileStatus
  hash?: string
  modified_at?: string
  size?: number
  node_type?: string
}

export interface SyncSummary {
  total_files: number
  synced: number
  modified: number
  local_only: number
  server_only: number
  conflicts: number
}

export interface SyncStatusResponse {
  package_name: string
  package_node_id: string
  summary: SyncSummary
  files: SyncFileInfo[]
  last_sync?: string
}

export interface FileDiff {
  path: string
  diff_type: 'unified' | 'side_by_side'
  local_content?: string
  server_content?: string
  unified_diff?: string
  local_hash?: string
  server_hash?: string
}

export interface ExportOptions {
  export_mode: 'filtered' | 'all'
  include_modifications: boolean
  filter_patterns?: string[]
}

export interface ExportResponse {
  job_id: string
  package_name: string
  export_mode: string
}

// ============================================
// Sync & Export API Functions
// ============================================

/**
 * Start exporting a package
 * Creates a job that generates a .rap file from the current package state
 */
export async function exportPackage(
  repo: string,
  name: string,
  options?: ExportOptions,
  branch = 'main'
): Promise<ExportResponse> {
  return api.post<ExportResponse>(
    `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:export`,
    options || { export_mode: 'all', include_modifications: true }
  )
}

/**
 * Download an exported package
 * Returns the URL to download the .rap file
 */
export function getExportDownloadUrl(
  repo: string,
  name: string,
  jobId: string,
  branch = 'main'
): string {
  return `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:download/${jobId}`
}

/**
 * Get sync status for a package
 * Returns file status information comparing local state with original package
 */
export async function getSyncStatus(
  repo: string,
  name: string,
  computeHashes = false,
  branch = 'main'
): Promise<SyncStatusResponse> {
  const query = computeHashes ? '?compute_hashes=true' : ''
  return api.get<SyncStatusResponse>(
    `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:sync-status${query}`
  )
}

/**
 * Get diff for a specific file in a package
 */
export async function getFileDiff(
  repo: string,
  name: string,
  filePath: string,
  branch = 'main'
): Promise<FileDiff> {
  return api.get<FileDiff>(
    `/api/packages/${repo}/${branch}/head/${encodeURIComponent(name)}/raisin:diff/${filePath}`
  )
}

/**
 * Selected path for package creation
 */
export interface SelectedPath {
  workspace: string
  path: string
}

/**
 * Request body for creating a package from selection
 */
export interface CreateFromSelectionRequest {
  name: string
  version?: string
  selected_paths: SelectedPath[]
  include_node_types?: boolean
  title?: string
  description?: string
  author?: string
}

/**
 * Response from create-from-selection endpoint
 */
export interface CreateFromSelectionResponse {
  job_id: string
  status: string
  download_path: string
  selected_count: number
}

/**
 * Create a new package from selected content paths
 * Creates a job that generates a .rap file from the selected nodes
 */
export async function createPackageFromSelection(
  repo: string,
  request: CreateFromSelectionRequest,
  branch = 'main'
): Promise<CreateFromSelectionResponse> {
  return api.post<CreateFromSelectionResponse>(
    `/api/packages/${repo}/${branch}/head/raisin:create-from-selection`,
    request
  )
}

// Convenience exports
export const packagesApi = {
  listPackages,
  getPackage,
  uploadPackage,
  installPackage,
  dryRunPackage,
  uninstallPackage,
  browsePackageContents,
  getPackageFile,
  deletePackage,
  exportPackage,
  getExportDownloadUrl,
  getSyncStatus,
  getFileDiff,
  createPackageFromSelection,
}
