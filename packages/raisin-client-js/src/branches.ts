/**
 * Branch management operations
 */

import { RequestContext, RequestType } from './protocol';

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
  async compare(branch: string, baseBranch: string): Promise<unknown> {
    return this.sendRequest(
      {
        branch,
        base_branch: baseBranch
      },
      RequestType.BranchCompare
    );
  }
}
