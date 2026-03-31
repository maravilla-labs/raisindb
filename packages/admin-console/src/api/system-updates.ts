import { api } from './client'

/** Breaking change types that can occur when updating NodeTypes */
export type BreakingChangeType =
  | 'PropertyRemoved'
  | 'PropertyTypeChanged'
  | 'RequiredAdded'
  | 'AllowedChildrenRemoved'
  | 'MixinRemoved'
  | 'AllowedNodeTypeRemoved'
  | 'AllowedRootNodeTypeRemoved'

/** Information about a single breaking change */
export interface BreakingChange {
  change_type: BreakingChangeType
  description: string
  path: string
}

/** Information about a single pending update */
export interface PendingUpdateInfo {
  /** Type of resource (NodeType, Workspace, or Package) */
  resource_type: 'NodeType' | 'Workspace' | 'Package'
  /** Name of the resource */
  name: string
  /** Whether this is a new resource (never applied) */
  is_new: boolean
  /** Whether this update contains breaking changes */
  is_breaking: boolean
  /** Number of breaking changes */
  breaking_count: number
  /** New version (if available) */
  new_version: number | null
  /** Currently applied version (if available) */
  old_version: number | null
}

/** Response for pending updates check */
export interface PendingUpdatesResponse {
  /** Whether there are any pending updates */
  has_updates: boolean
  /** Total number of pending updates */
  total_pending: number
  /** Number of updates with breaking changes */
  breaking_count: number
  /** List of pending updates */
  updates: PendingUpdateInfo[]
}

/** Request to apply system updates */
export interface ApplyUpdatesRequest {
  /** Specific resources to update (empty = all pending) */
  resources?: string[]
  /** Force apply even with breaking changes */
  force?: boolean
}

/** Response for apply updates request */
export interface ApplyUpdatesResponse {
  /** Job ID for tracking the update (if async) */
  job_id: string | null
  /** Message describing the result */
  message: string
  /** Number of updates applied */
  applied_count: number
  /** Number of updates skipped (due to breaking changes) */
  skipped_count: number
}

/** System Updates API client */
export const systemUpdatesApi = {
  /**
   * Get pending system updates for a repository
   *
   * @param tenant - Tenant ID
   * @param repo - Repository ID
   * @returns Summary of pending updates including breaking change detection
   */
  getPending: (tenant: string, repo: string) =>
    api.get<PendingUpdatesResponse>(
      `/api/management/repositories/${tenant}/${repo}/system-updates`
    ),

  /**
   * Apply pending system updates to a repository
   *
   * @param tenant - Tenant ID
   * @param repo - Repository ID
   * @param request - Optional request body specifying which resources to update and force flag
   * @returns Result of the apply operation
   */
  apply: (tenant: string, repo: string, request?: ApplyUpdatesRequest) =>
    api.post<ApplyUpdatesResponse>(
      `/api/management/repositories/${tenant}/${repo}/system-updates/apply`,
      request ?? {}
    ),

  /**
   * Apply specific updates by name
   *
   * @param tenant - Tenant ID
   * @param repo - Repository ID
   * @param resources - Array of resource names to update
   * @param force - Force apply even with breaking changes
   */
  applySelected: (
    tenant: string,
    repo: string,
    resources: string[],
    force: boolean = false
  ) =>
    api.post<ApplyUpdatesResponse>(
      `/api/management/repositories/${tenant}/${repo}/system-updates/apply`,
      { resources, force }
    ),

  /**
   * Apply all pending updates (convenience method)
   *
   * @param tenant - Tenant ID
   * @param repo - Repository ID
   * @param force - Force apply even with breaking changes
   */
  applyAll: (tenant: string, repo: string, force: boolean = false) =>
    api.post<ApplyUpdatesResponse>(
      `/api/management/repositories/${tenant}/${repo}/system-updates/apply`,
      { resources: [], force }
    ),
}
