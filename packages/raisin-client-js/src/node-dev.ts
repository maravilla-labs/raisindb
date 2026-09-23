/**
 * The node-development surface over HTTP: RaisinDB's hierarchy as a
 * structured workspace a client — or an agent — can develop in safely.
 *
 * - **Locators** name a node as `{repository, branch, workspace, path,
 *   node_id, revision}`; the id is identity across moves, the revision is
 *   what you saw.
 * - **Roots** bound every call: a path outside them (including a `..` climb)
 *   is refused, and a root can restrict the operations allowed inside it.
 * - **Changesets** are atomic: {@link NodeDevApi.dryRun} plans,
 *   {@link NodeDevApi.propose} stores a reviewable record with a digest,
 *   {@link NodeDevApi.commit} applies it (optionally bound to the approved
 *   digest), and {@link NodeDevApi.apply} does both. A stale
 *   `expected_revision` comes back as a `conflict` result — nothing is
 *   written — and an `idempotency_key` makes a retry return the first receipt.
 * - **Branches** are worktrees: fork, diff, merge (conflicts reported, never
 *   guessed), discard.
 */

/** A subtree of one workspace, and the operations allowed inside it. */
export interface WorkRoot {
  workspace: string;
  /** Absolute subtree path (default `/`). */
  path?: string;
  /** Allowed operations (default: all). */
  ops?: Array<'read' | 'create' | 'update' | 'delete' | 'move' | 'copy'>;
}

/** A node revision under a named scheme. */
export interface NodeRevision {
  value: string;
  alg: string;
}

/** A canonical locator. */
export interface NodeLocator {
  repository: string;
  branch: string;
  workspace: string;
  path: string;
  node_id?: string;
  revision?: NodeRevision;
}

/** A path (relative to the first root, or absolute), or `{ path?, node_id?, workspace? }`. */
export type NodeTarget = string | { workspace?: string; path?: string; node_id?: string };

/** One changeset operation. */
export type ChangeOp =
  | {
      op: 'create';
      path: string;
      node_type: string;
      workspace?: string;
      archetype?: string;
      properties?: Record<string, unknown>;
    }
  | {
      op: 'patch';
      target: NodeTarget;
      expected_revision?: string | NodeRevision;
      set?: Record<string, unknown>;
      unset?: string[];
      archetype?: string;
    }
  | { op: 'move'; target: NodeTarget; to_parent: NodeTarget; new_name?: string; expected_revision?: string | NodeRevision }
  | { op: 'rename'; target: NodeTarget; new_name: string; expected_revision?: string | NodeRevision }
  | { op: 'copy'; source: NodeTarget; to_parent: NodeTarget; new_name?: string }
  | { op: 'delete'; target: NodeTarget; expected_revision?: string | NodeRevision; recursive?: boolean };

/** Fields every call accepts. */
export interface NodeDevCommon {
  /** The working roots (or `workspace` for a whole workspace). */
  roots?: WorkRoot[];
  workspace?: string;
  /** Branch (default `main`). */
  branch?: string;
  /** Return a `raisin.tool-result/1` envelope instead of the typed result. */
  envelope?: boolean;
}

/** A changeset request. */
export interface ChangesetOptions extends NodeDevCommon {
  ops: ChangeOp[];
  idempotencyKey?: string;
  message?: string;
  /** Index of the op whose node is the primary artifact. */
  primary?: number;
  /** Artifact kind reported in tool results. */
  kind?: string;
}

/** Why an op cannot commit as reviewed. */
export interface NodeDevConflict {
  index: number;
  code: string;
  message: string;
  expected?: string;
  actual?: NodeLocator;
}

/** One op's committed effect. */
export interface OpReceipt {
  index: number;
  action: 'created' | 'updated' | 'moved' | 'copied' | 'deleted';
  old?: NodeLocator;
  new?: NodeLocator;
  changed_properties?: string[];
  created_descendants?: NodeLocator[];
  deleted_descendants?: NodeLocator[];
  moved_descendants?: Array<{ from_path: string; to: NodeLocator }>;
  rewritten_references?: Array<{ workspace: string; node_id: string; path?: string; property: string; target_id: string }>;
}

/** Exactly what committed. */
export interface Receipt {
  changeset_id: string;
  repository: string;
  branch: string;
  committed_revision?: string;
  ops: OpReceipt[];
  replayed: boolean;
}

/** The outcome of a commit or apply. */
export type CommitOutcome =
  | { result: 'committed'; receipt: Receipt }
  | { result: 'conflict'; changeset_id: string; conflicts: NodeDevConflict[]; digest: string };

/** How requests reach the server (the HTTP client provides it). */
export interface NodeDevTransport {
  request<T>(method: string, path: string, body?: unknown): Promise<T>;
}

/** The node-development surface of one repository. */
export class NodeDevApi {
  constructor(
    private readonly repository: string,
    private readonly transport: NodeDevTransport,
  ) {}

  /** Call any method by name (the typed helpers below all use this). */
  call<T = unknown>(method: string, request: Record<string, unknown>): Promise<T> {
    const branch = typeof request.branch === 'string' ? `?branch=${encodeURIComponent(request.branch)}` : '';
    return this.transport.request<T>(
      'POST',
      `/api/node-dev/${encodeURIComponent(this.repository)}/${method}${branch}`,
      request,
    );
  }

  private changeset(o: ChangesetOptions): Record<string, unknown> {
    const { idempotencyKey, ...rest } = o;
    return { ...rest, idempotency_key: idempotencyKey };
  }

  stat(target: NodeTarget, o: NodeDevCommon = {}) {
    return this.call('stat', { ...o, target });
  }
  read(target: NodeTarget, o: NodeDevCommon & { keys?: string[] } = {}) {
    return this.call('read', { ...o, target });
  }
  list(target?: NodeTarget, o: NodeDevCommon & { limit?: number } = {}) {
    return this.call('list', { ...o, target });
  }
  diff(target: NodeTarget, from: { branch?: string; revision?: string }, to: { branch?: string; revision?: string }, o: NodeDevCommon = {}) {
    return this.call('diff', { ...o, target, from, to });
  }
  /** Changes inside the roots after `since` (a cursor from a previous call). */
  watch(since?: string, o: NodeDevCommon & { limit?: number } = {}) {
    return this.call<{ changes: unknown[]; cursor?: string }>('watch', { ...o, since });
  }

  dryRun(o: ChangesetOptions) {
    return this.call('dry_run', this.changeset(o));
  }
  propose(o: ChangesetOptions) {
    return this.call<{ changeset: { changeset_id: string; plan: { digest: string } }; created: boolean }>(
      'propose',
      this.changeset(o),
    );
  }
  getChangeset(changesetId: string, o: NodeDevCommon = {}) {
    return this.call('get_changeset', { ...o, changeset_id: changesetId });
  }
  listChangesets(o: NodeDevCommon & { status?: 'proposed' | 'committed' | 'discarded'; limit?: number } = {}) {
    return this.call('list_changesets', { ...o });
  }
  /** Commit; pass the reviewed digest to bind the commit to that approval. */
  commit(changesetId: string, expectedDigest?: string, o: NodeDevCommon = {}) {
    return this.call<CommitOutcome>('commit', { ...o, changeset_id: changesetId, expected_digest: expectedDigest });
  }
  discard(changesetId: string, o: NodeDevCommon = {}) {
    return this.call('discard', { ...o, changeset_id: changesetId });
  }
  /** Propose and commit in one call. */
  apply(o: ChangesetOptions) {
    return this.call<CommitOutcome>('apply', this.changeset(o));
  }

  forkBranch(name: string, o: NodeDevCommon = {}) {
    return this.call('fork_branch', { ...o, name });
  }
  diffBranch(base = 'main', o: NodeDevCommon = {}) {
    return this.call('diff_branch', { ...o, base });
  }
  mergeBranch(target: string, o: NodeDevCommon & { dryRun?: boolean; message?: string } = {}) {
    const { dryRun, ...rest } = o;
    return this.call('merge_branch', { ...rest, target, dry_run: dryRun });
  }
  discardBranch(o: NodeDevCommon = {}) {
    return this.call('discard_branch', { ...o });
  }
}
