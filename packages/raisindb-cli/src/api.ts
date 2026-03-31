import { getToken } from './auth.js';
import { loadConfig } from './config.js';
import { EventSource as EventSourcePolyfill } from 'eventsource';
import { computeHash, ServerFileInfo } from './sync/compare.js';

export interface SqlResult {
  columns: string[];
  rows: Record<string, unknown>[];
  row_count: number;
  execution_time_ms?: number;
}

export interface Repository {
  tenant_id: string;
  repo_id: string;
  created_at: string;
  branches: string[];
  config: {
    default_branch: string;
    description?: string;
    default_language: string;
    supported_languages: string[];
  };
}

export interface PackageSummary {
  id: string;
  name: string;
  version: string;
  title?: string;
  installed: boolean;
}

export interface NodeInfo {
  id: string;
  name: string;
  node_type: string;
  parent_id?: string;
  path?: string;
  has_children?: boolean;
  properties?: Record<string, unknown>;
  created_at?: string;
  updated_at?: string;
}

export interface Workspace {
  name: string;
  description?: string;
}

export interface PackageUploadResult {
  name: string;
  version: string;
  /** Job ID for tracking background processing (large uploads) */
  job_id?: string;
  /** Node ID of the created package */
  node_id?: string;
  /** Status: 'processing' for large uploads */
  status?: string;
}

export interface JobEvent {
  job_id: string;
  job_type: string;
  status: string;
  old_status: string | null;
  progress: number | null;
  error: string | null;
  timestamp: string;
}

/**
 * Get the base URL for API calls
 */
export function getBaseUrl(): string {
  const config = loadConfig();
  return config.server || 'http://localhost:8081';
}

/**
 * Get auth headers
 */
export function getHeaders(): Record<string, string> {
  const token = getToken();
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  if (token) {
    headers['Authorization'] = `Bearer ${token}`;
  }
  return headers;
}

/**
 * Execute a SQL query against the RaisinDB server
 */
export async function executeSql(repo: string, query: string): Promise<SqlResult> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/sql/${repo}`;

  const response = await fetch(url, {
    method: 'POST',
    headers: getHeaders(),
    body: JSON.stringify({ sql: query }),  // Backend expects 'sql' field
  });

  if (!response.ok) {
    const errorText = await response.text();
    let errorMessage: string;
    try {
      const errorData = JSON.parse(errorText) as { message?: string; error?: string };
      errorMessage = errorData.message || errorData.error || `SQL query failed: ${response.status}`;
    } catch {
      errorMessage = errorText || `SQL query failed: ${response.status} ${response.statusText}`;
    }
    throw new Error(errorMessage);
  }

  return response.json() as Promise<SqlResult>;
}

/**
 * List repositories
 */
export async function listRepositories(): Promise<Repository[]> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/repositories`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to list repositories: ${response.status}`);
  }

  return response.json() as Promise<Repository[]>;
}

/**
 * Raw node response from API
 */
interface PackageNodeResponse {
  id: string;
  name: string;
  node_type: string;
  properties?: Record<string, unknown>;
}

/**
 * List packages in a repository
 */
export async function listPackages(repo: string): Promise<PackageSummary[]> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/repos/${repo}/packages`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to list packages: ${response.status}`);
  }

  // API returns full Node objects, transform to PackageSummary
  const nodes = await response.json() as PackageNodeResponse[];
  return nodes.map(node => ({
    id: node.id,
    name: node.name,
    version: (node.properties?.version as string) || '0.0.0',
    title: node.properties?.title as string | undefined,
    installed: (node.properties?.installed as boolean) ?? false,
  }));
}

/**
 * Upload a package file using the unified endpoint
 *
 * Uses: POST /api/repository/{repo}/main/head/packages/{path}?nodeType=raisin:Package
 *
 * Creates a raisin:Package node with status "processing". A background job
 * will extract the manifest and update the node properties.
 *
 * @param repo - Repository name
 * @param fileContent - Package file content as Buffer
 * @param fileName - Original filename (e.g., "my-package-1.0.0.rap")
 * @param targetPath - Optional target path (e.g., "/my-folder/my-package"). Defaults to package name from filename.
 */
export async function uploadPackage(repo: string, fileContent: Buffer, fileName: string, targetPath?: string): Promise<PackageUploadResult> {
  const baseUrl = getBaseUrl();

  // Extract package name from filename (remove .rap extension)
  const packageName = fileName.replace(/\.rap$/, '');

  // Use targetPath if provided, otherwise default to package name
  // Remove leading slash if present since we add it in the URL
  const nodePath = targetPath
    ? targetPath.replace(/^\//, '')
    : packageName;

  // Use unified endpoint with nodeType parameter (camelCase per RepoQuery struct)
  const url = `${baseUrl}/api/repository/${repo}/main/head/packages/${encodeURIComponent(nodePath)}?nodeType=raisin:Package`;

  const formData = new FormData();
  formData.append('file', new Blob([fileContent]), fileName);

  const token = getToken();
  const headers: Record<string, string> = {};
  if (token) {
    headers['Authorization'] = `Bearer ${token}`;
  }

  const response = await fetch(url, {
    method: 'POST',
    headers,
    body: formData,
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to upload package: ${response.status}`);
  }

  const result = await response.json() as {
    storedKey: string;
    url: string;
    node_id?: string;
    job_id?: string;
    status?: string;
  };

  return {
    name: packageName,
    version: 'processing', // Will be updated by background job
    job_id: result.job_id,
    node_id: result.node_id,
    status: result.status,
  };
}

/**
 * Subscribe to job events via SSE
 * Returns a cleanup function to close the connection
 */
export function subscribeToJobEvents(
  onEvent: (event: JobEvent) => void,
  onError?: (error: Error) => void
): () => void {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/management/events/jobs`;

  // Using eventsource polyfill for Node.js (EventSource is browser-only)
  const eventSource = new EventSourcePolyfill(url);

  // Add event listener with proper typing
  const handler = (event: Event) => {
    try {
      const messageEvent = event as MessageEvent;
      const data = JSON.parse(messageEvent.data) as JobEvent;
      onEvent(data);
    } catch (e) {
      // Ignore parse errors
    }
  };

  eventSource.addEventListener('job-update', handler);

  eventSource.onerror = () => {
    onError?.(new Error('SSE connection error'));
  };

  return () => {
    eventSource.removeEventListener('job-update', handler);
    eventSource.close();
  };
}

/**
 * Install a package
 */
export async function installPackage(repo: string, packageName: string): Promise<void> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/repos/${repo}/packages/${encodeURIComponent(packageName)}/install`;

  const response = await fetch(url, {
    method: 'POST',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to install package: ${response.status}`);
  }
}

/**
 * Uninstall a package
 */
export async function uninstallPackage(repo: string, packageName: string): Promise<void> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/repos/${repo}/packages/${encodeURIComponent(packageName)}/uninstall`;

  const response = await fetch(url, {
    method: 'POST',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to uninstall package: ${response.status}`);
  }
}

interface PagedResponse<T> {
  items: T[];
  page: {
    total: number;
    limit: number;
    offset: number;
    next_offset?: number;
  };
}

/**
 * List workspaces in a repository
 */
export async function listWorkspaces(repo: string): Promise<Workspace[]> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/workspaces/${repo}`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to list workspaces: ${response.status}`);
  }

  const data = await response.json() as PagedResponse<Workspace>;
  return data.items;
}

/**
 * List child nodes at a given path using REST API
 * GET /api/repository/{repo}/{branch}/head/{workspace}/{path}/
 */
export async function listNodes(repo: string, workspace: string, parentPath: string = '/'): Promise<NodeInfo[]> {
  const baseUrl = getBaseUrl();
  // Build path: root is empty, otherwise append path without leading slash
  // Always add trailing slash as the API expects it
  const pathPart = parentPath === '/' ? '' : parentPath.replace(/^\//, '');
  const url = `${baseUrl}/api/repository/${repo}/main/head/${workspace}/${pathPart}${pathPart ? '/' : ''}`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorText = await response.text();
    let errorMessage: string;
    try {
      const errorData = JSON.parse(errorText) as { message?: string; error?: string };
      errorMessage = errorData.message || errorData.error || `Failed to list nodes: ${response.status}`;
    } catch {
      errorMessage = errorText || `Failed to list nodes: ${response.status}`;
    }
    throw new Error(errorMessage);
  }

  // API returns plain array of nodes
  const nodes = await response.json() as Array<{
    id: string;
    name: string;
    node_type: string;
    path: string;
    has_children: boolean;
  }>;

  return nodes.map(node => ({
    id: node.id,
    name: node.name,
    node_type: node.node_type,
    path: node.path,
    has_children: node.has_children,
  }));
}

/**
 * Get node by path - fetches parent's children and finds the node
 */
export async function getNodeByPath(repo: string, workspace: string, path: string): Promise<NodeInfo | null> {
  // Normalize path
  const normalizedPath = path.startsWith('/') ? path : `/${path}`;
  const segments = normalizedPath.split('/').filter(s => s.length > 0);

  if (segments.length === 0) {
    return null; // Root doesn't have a node
  }

  // Get parent path and target name
  const targetName = segments[segments.length - 1];
  const parentPath = segments.length === 1 ? '/' : '/' + segments.slice(0, -1).join('/');

  // List parent's children and find the target
  const children = await listNodes(repo, workspace, parentPath);
  const node = children.find(n => n.name === targetName);

  return node || null;
}

/**
 * Get full node data by path (without trailing slash returns the node itself)
 */
export async function getNodeFull(repo: string, workspace: string, path: string): Promise<Record<string, unknown> | null> {
  const baseUrl = getBaseUrl();
  // Build path without trailing slash to get the node itself
  const pathPart = path.replace(/^\//, '');
  const url = `${baseUrl}/api/repository/${repo}/main/head/${workspace}/${pathPart}`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    if (response.status === 404) {
      return null;
    }
    const errorText = await response.text();
    let errorMessage: string;
    try {
      const errorData = JSON.parse(errorText) as { message?: string; error?: string };
      errorMessage = errorData.message || errorData.error || `Failed to get node: ${response.status}`;
    } catch {
      errorMessage = errorText || `Failed to get node: ${response.status}`;
    }
    throw new Error(errorMessage);
  }

  return response.json() as Promise<Record<string, unknown>>;
}

// =========================================================================
// Authentication API Functions
// =========================================================================

export interface AuthProvider {
  id: string;
  display_name: string;
  icon: string;
  auth_url: string;
}

export interface AuthProvidersResponse {
  providers: AuthProvider[];
  local_enabled: boolean;
  magic_link_enabled: boolean;
}

export interface SessionInfo {
  id: string;
  auth_strategy: string;
  user_agent?: string;
  ip_address?: string;
  created_at: string;
  last_active_at: string;
  is_current: boolean;
}

export interface SessionsResponse {
  sessions: SessionInfo[];
}

export interface IdentityInfo {
  id: string;
  email: string;
  display_name?: string;
  avatar_url?: string;
  email_verified: boolean;
  linked_providers: string[];
}

/**
 * Get available authentication providers
 */
export async function getAuthProviders(): Promise<AuthProvidersResponse> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/auth/providers`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to get auth providers: ${response.status}`);
  }

  return response.json() as Promise<AuthProvidersResponse>;
}

/**
 * Get current user's sessions
 */
export async function getAuthSessions(): Promise<SessionsResponse> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/auth/sessions`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to get sessions: ${response.status}`);
  }

  return response.json() as Promise<SessionsResponse>;
}

/**
 * Revoke a specific session
 */
export async function revokeSession(sessionId: string): Promise<void> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/auth/sessions/${sessionId}`;

  const response = await fetch(url, {
    method: 'DELETE',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to revoke session: ${response.status}`);
  }
}

/**
 * Get current user identity information
 */
export async function getCurrentUser(): Promise<IdentityInfo> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/auth/me`;

  const response = await fetch(url, {
    method: 'GET',
    headers: getHeaders(),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to get current user: ${response.status}`);
  }

  return response.json() as Promise<IdentityInfo>;
}

// =========================================================================
// Package Export/Clone API Functions
// =========================================================================

export interface ExportOptions {
  export_mode?: 'all' | 'filtered';
  include_modifications?: boolean;
}

export interface ExportResponse {
  job_id: string;
  package_name: string;
  export_mode: string;
}

/**
 * Start exporting a package
 * Creates a job that generates a .rap file from the current package state
 */
export async function exportPackage(
  repo: string,
  packageName: string,
  options?: ExportOptions,
  branch = 'main'
): Promise<ExportResponse> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/packages/${repo}/${branch}/head/${encodeURIComponent(packageName)}/raisin:export`;

  const response = await fetch(url, {
    method: 'POST',
    headers: getHeaders(),
    body: JSON.stringify(options || { export_mode: 'all', include_modifications: true }),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to export package: ${response.status}`);
  }

  return response.json() as Promise<ExportResponse>;
}

/**
 * Get the download URL for an exported package
 */
export function getExportDownloadUrl(
  repo: string,
  packageName: string,
  jobId: string,
  branch = 'main'
): string {
  const baseUrl = getBaseUrl();
  return `${baseUrl}/api/packages/${repo}/${branch}/head/${encodeURIComponent(packageName)}/raisin:download/${jobId}`;
}

/**
 * Download an exported package file
 */
export async function downloadExportedPackage(
  repo: string,
  packageName: string,
  jobId: string,
  branch = 'main'
): Promise<ArrayBuffer> {
  const url = getExportDownloadUrl(repo, packageName, jobId, branch);

  const token = getToken();
  const headers: Record<string, string> = {};
  if (token) {
    headers['Authorization'] = `Bearer ${token}`;
  }

  const response = await fetch(url, {
    method: 'GET',
    headers,
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(errorText || `Failed to download package: ${response.status}`);
  }

  return response.arrayBuffer();
}

// =========================================================================
// Package Create from Selection API Functions
// =========================================================================

export interface SelectedPath {
  workspace: string;
  path: string;
}

export interface CreateFromSelectionRequest {
  name: string;
  version?: string;
  selected_paths: SelectedPath[];
  include_node_types?: boolean;
  title?: string;
  description?: string;
  author?: string;
}

export interface CreateFromSelectionResponse {
  job_id: string;
  status: string;
  download_path: string;
  selected_count: number;
}

/**
 * Create a new package from selected content paths
 */
export async function createPackageFromSelection(
  repo: string,
  request: CreateFromSelectionRequest,
  branch = 'main'
): Promise<CreateFromSelectionResponse> {
  const baseUrl = getBaseUrl();
  const url = `${baseUrl}/api/packages/${repo}/${branch}/head/raisin:create-from-selection`;

  const response = await fetch(url, {
    method: 'POST',
    headers: getHeaders(),
    body: JSON.stringify(request),
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: response.statusText })) as { message?: string };
    throw new Error(errorData.message || `Failed to create package: ${response.status}`);
  }

  return response.json() as Promise<CreateFromSelectionResponse>;
}

// =========================================================================
// Sync API Functions
// =========================================================================

/**
 * Server node from deep query API
 */
interface ServerNode {
  id: string;
  name: string;
  node_type: string;
  path: string;
  properties?: Record<string, unknown>;
  updated_at?: string;
}

/**
 * Get server files for sync comparison across multiple workspaces
 * Uses deep query API to fetch all files from each workspace root
 *
 * @param repo - Repository name
 * @param workspaces - List of workspace names to fetch (e.g., ['functions', 'content'])
 * @param branch - Branch name (default: 'main')
 * @returns Map with keys as "{workspace}/{path}" (e.g., "functions/lib/weather/index.js")
 */
export async function getServerFilesForWorkspaces(
  repo: string,
  workspaces: string[],
  branch: string = 'main'
): Promise<Map<string, ServerFileInfo>> {
  const baseUrl = getBaseUrl();
  const allFiles = new Map<string, ServerFileInfo>();

  for (const workspace of workspaces) {
    // Fetch ALL content from workspace root with level=10 and flatten=true
    const url = `${baseUrl}/api/repository/${repo}/${branch}/head/${workspace}/?level=10&flatten=true`;

    const response = await fetch(url, {
      method: 'GET',
      headers: getHeaders(),
    });

    if (!response.ok) {
      // Workspace might not exist on server - log and continue
      if (response.status === 404) {
        console.log(`Workspace '${workspace}' not found on server, skipping`);
        continue;
      }
      const errorText = await response.text();
      let errorMessage: string;
      try {
        const errorData = JSON.parse(errorText) as { message?: string; error?: string };
        errorMessage = errorData.message || errorData.error || `Failed to get server files: ${response.status}`;
      } catch {
        errorMessage = errorText || `Failed to get server files: ${response.status}`;
      }
      throw new Error(errorMessage);
    }

    // API returns an array of nodes when flatten=true
    const nodes = await response.json() as ServerNode[];

    for (const node of nodes) {
      const nodePath = node.path || '/';

      // Skip folders - they don't have content to hash
      if (node.node_type?.endsWith(':Folder') || node.node_type === 'raisin:Folder') {
        continue;
      }

      // Skip workspace root itself
      if (nodePath === '/') {
        continue;
      }

      // Compute hash from node content
      // For function nodes, use the code property; otherwise hash all properties
      let content: string;
      if (node.properties?.code && typeof node.properties.code === 'string') {
        content = node.properties.code;
      } else {
        // Hash the serialized properties for other node types
        content = JSON.stringify(node.properties || {});
      }
      const hash = computeHash(content);

      // Full path includes workspace: "functions/lib/weather/index.js"
      // nodePath from API starts with "/" like "/lib/weather/index.js"
      const fullPath = `${workspace}${nodePath}`;

      allFiles.set(fullPath, {
        path: fullPath,
        hash,
        modified_at: node.updated_at || new Date().toISOString(),
        node_type: node.node_type,
      });
    }
  }

  return allFiles;
}

