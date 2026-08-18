/**
 * Branch management operations
 */

import { RequestContext, RequestType } from './protocol';

/**
 * Branch divergence information (similar to "N commits ahead/behind").
 */
export interface BranchDivergence {
  /** Number of commits on the compared branch not on the base branch */
  ahead: number;
  /** Number of commits on the base branch not on the compared branch */
  behind: number;
  /** Common ancestor revision (HLC string, e.g. "1705584000000-5") */
  common_ancestor: string;
}

/**
 * Per-node change in a branch diff (relative to the merge-base).
 */
export interface NodeDiffInfo {
  /** Node id that changed */
  node_id: string;
  /** Workspace the node belongs to */
  workspace: string;
  /** Path on the diffed branch (null when only a tombstone exists, e.g. some deletes) */
  path: string | null;
  /** Operation: "added" | "modified" | "deleted" | "reordered" */
  operation: string;
  /** Translation locale when the change is to a translation (absent = base node) */
  translation_locale?: string;
}

/**
 * Per-node diff of a branch relative to a base branch's merge-base.
 *
 * Unlike {@link BranchDivergence} (ahead/behind counts only), this
 * enumerates exactly which nodes changed since the two branches diverged.
 */
export interface BranchDiff {
  /** The common ancestor (merge-base) the diff is computed against (HLC string) */
  common_ancestor: string;
  /** Nodes present on the branch but not on the base */
  added: NodeDiffInfo[];
  /** Nodes changed on the branch (operation "modified" / "reordered") */
  modified: NodeDiffInfo[];
  /** Nodes removed on the branch */
  deleted: NodeDiffInfo[];
}

/**
 * Per-node change from a cross-branch node-set copy.
 */
export interface CrossBranchNodeChange {
  /** Node id (ids are preserved across branches) */
  node_id: string;
  /** Path of the node on the target branch */
  path: string;
  /** NodeType of the node */
  node_type: string;
  /** Operation on the target branch: "added" | "modified" | "deleted" */
  operation: string;
}

/**
 * Summary of a cross-branch node-set copy (see {@link Branches.copyNodes}).
 */
export interface CrossBranchCopySummary {
  /** Number of nodes written to the target branch (added + modified) */
  copied: number;
  /** Number of target-branch nodes pruned (deleteMissing only) */
  deleted: number;
  /** The single revision all changes were committed under (HLC string) */
  revision: string;
  /** Per-node change list */
  changes: CrossBranchNodeChange[];
}

export class Branches {
  private sendRequest: (payload: unknown, requestType: string) => Promise<unknown>;

  constructor(
    _context: RequestContext,
    sendRequest: (payload: unknown, requestType: string) => Promise<unknown>
  ) {
    this.sendRequest = sendRequest;
  }

  /**
   * Create a new branch
   */
  async create(
    name: string,
    options?: {
      fromRevision?: string;
      fromBranch?: string;
    }
  ): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        from_revision: options?.fromRevision,
        from_branch: options?.fromBranch
      },
      RequestType.BranchCreate
    );
  }

  /**
   * Get a branch by name
   */
  async get(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.BranchGet
    );
  }

  /**
   * List all branches
   */
  async list(): Promise<unknown[]> {
    const result = await this.sendRequest(
      {},
      RequestType.BranchList
    );
    return Array.isArray(result) ? result : [];
  }

  /**
   * Delete a branch
   */
  async delete(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.BranchDelete
    );
  }

  /**
   * Get the HEAD revision of a branch
   */
  async getHead(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.BranchGetHead
    );
  }

  /**
   * Update the HEAD revision of a branch
   */
  async updateHead(name: string, revision: string): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        revision
      },
      RequestType.BranchUpdateHead
    );
  }

  /**
   * Merge a source branch into a target branch
   */
  async merge(
    sourceBranch: string,
    targetBranch: string,
    options?: {
      strategy?: string;
      message?: string;
    }
  ): Promise<unknown> {
    return this.sendRequest(
      {
        source_branch: sourceBranch,
        target_branch: targetBranch,
        strategy: options?.strategy,
        message: options?.message
      },
      RequestType.BranchMerge
    );
  }

  /**
   * Compare two branches to calculate divergence
   */
  async compare(branch: string, baseBranch: string): Promise<BranchDivergence> {
    return this.sendRequest(
      {
        branch,
        base_branch: baseBranch
      },
      RequestType.BranchCompare
    ) as Promise<BranchDivergence>;
  }

  /**
   * Per-node diff of `branch` relative to `baseBranch` (added/modified/deleted).
   *
   * Cheaper than reading both branch listings: cost scales with the number of
   * changed nodes since the branches diverged, not the total content.
   */
  async diff(branch: string, baseBranch: string): Promise<BranchDiff> {
    return this.sendRequest(
      {
        branch,
        base_branch: baseBranch
      },
      RequestType.BranchDiff
    ) as Promise<BranchDiff>;
  }

  /**
   * Copy a node set from `sourceBranch` onto `targetBranch` (branch
   * promotion): node ids are preserved, so repeated copies update the same
   * target nodes; all changes land in ONE atomic commit on the target branch.
   *
   * Each root's parent path must already exist on the target branch. With
   * `deleteMissing`, target nodes under the roots that are absent from the
   * copied source set are pruned (one-way sync).
   *
   * `sourceRevision` pins the read to a point in time. A promotion of a large
   * set is not instantaneous, so without it the copy reads each node when its
   * turn comes and a writer working in parallel lands partly inside the result
   * — a torn snapshot. Pass the revision captured when the operation was
   * decided (see `getHead`) to copy exactly that state. The target branch is
   * always resolved at its head: the pin says WHAT to copy, never where.
   */
  async copyNodes(
    sourceBranch: string,
    targetBranch: string,
    opts: {
      workspace: string;
      roots: string[];
      recursive?: boolean;
      deleteMissing?: boolean;
      sourceRevision?: string;
    }
  ): Promise<CrossBranchCopySummary> {
    return this.sendRequest(
      {
        source_branch: sourceBranch,
        target_branch: targetBranch,
        workspace: opts.workspace,
        roots: opts.roots,
        recursive: opts.recursive ?? true,
        delete_missing: opts.deleteMissing ?? false,
        source_revision: opts.sourceRevision ?? null
      },
      RequestType.BranchCopyNodes
    ) as Promise<CrossBranchCopySummary>;
  }
}
