/**
 * `raisindb function run` — one local artifact, one server, one result.
 *
 * The orchestration lives here and emits events rather than printing, so the
 * same code path serves the Ink UI (`components/FunctionRun.tsx`), `--json`
 * and `function test --server`, which runs many cases in a row.
 */

import crypto from 'crypto';
import fs from 'fs';
import { getToken } from '../auth.js';
import { getDefaultRepo, getServer } from '../config.js';
import { loadSyncConfig } from '../sync/config.js';
import { toHttpUrl } from '../sync/operations.js';
import { formatBytes } from './build.js';
import {
  getArtifactNode,
  getFunctionDetails,
  invokeFunction,
  streamRunFile,
  uploadArtifact,
  type RunOutcome,
  type ServerContext,
} from './run-client.js';
import { planRun, workspaceLocation, type RunPlan, type RunTarget } from './run-target.js';
import { MAX_ARTIFACT_BYTES } from './types.js';

/** Progress the caller renders however it likes. */
export type RunEvent =
  | { kind: 'phase'; message: string }
  | { kind: 'log'; level: string; message: string }
  | { kind: 'result'; outcome: RunOutcome };

/** Server-selection options shared by `run` and `test --server`. */
export interface ServerOptions {
  server?: string;
  repo?: string;
  branch?: string;
}

/** Everything `executeRun` accepts beyond the target. */
export interface ExecuteRunOptions extends ServerOptions {
  /** JSON input for the handler. */
  input: unknown;
  /** Per-run timeout in milliseconds. */
  timeoutMs?: number;
  /** Injected for tests; defaults to the global `fetch`. */
  fetchImpl?: typeof fetch;
}

/**
 * Resolve where to talk to, in the order the rest of the CLI does: explicit
 * flags, then the package's `.raisindb-cli.yaml`, then `~/.raisinrc`.
 */
export function resolveServerContext(dir: string, options: ServerOptions = {}): ServerContext {
  const sync = loadSyncConfig(dir);
  const server = options.server || sync?.server || getServer() || 'http://localhost:8081';
  const repo = options.repo || sync?.repository || getDefaultRepo();
  if (!repo) {
    throw new Error(
      'No repository. Pass --repo, set one in .raisindb-cli.yaml, or run `raisindb repo use <name>`.'
    );
  }
  return {
    baseUrl: toHttpUrl(server).replace(/\/$/, ''),
    repo,
    branch: options.branch || sync?.branch || 'main',
    token: getToken(),
    fetchImpl: fetch,
  };
}

/** Hex sha256 of a buffer, the spelling the server's `content_hash` uses. */
export function sha256Hex(bytes: Uint8Array): string {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

/** Read the artifact, refusing what the server would refuse at upload. */
function readArtifact(target: RunTarget): Buffer {
  if (!fs.existsSync(target.artifactPath)) {
    throw new Error(
      `${target.artifactPath} does not exist — run \`raisindb function build\` first.`
    );
  }
  const bytes = fs.readFileSync(target.artifactPath);
  if (bytes.length > MAX_ARTIFACT_BYTES) {
    throw new Error(
      `${target.artifactPath} is ${formatBytes(bytes.length)}, over the server's ` +
        `${formatBytes(MAX_ARTIFACT_BYTES)} artifact limit.`
    );
  }
  return bytes;
}

/**
 * Run one function against a server.
 *
 * Returns the outcome; it throws only for problems that are not the function's
 * own failure (no artifact, no repo, an HTTP error on the way there).
 */
export async function executeRun(
  target: RunTarget,
  options: ExecuteRunOptions,
  emit: (event: RunEvent) => void = () => {}
): Promise<{ outcome: RunOutcome; plan: RunPlan }> {
  const ctx = { ...resolveServerContext(target.node.dir, options) };
  if (options.fetchImpl) ctx.fetchImpl = options.fetchImpl;

  const bytes = readArtifact(target);
  const localHash = sha256Hex(bytes);
  const location = workspaceLocation(target.packageRoot, target.artifactPath);

  emit({
    kind: 'phase',
    message: `${target.node.name} -> ${ctx.baseUrl} (${ctx.repo}/${ctx.branch}), handler "${target.handler}"`,
  });
  if (ctx.branch !== 'main') {
    // `find_function_node` and `find_asset_node_by_id` both resolve on
    // DEFAULT_BRANCH, so an artifact uploaded elsewhere is invisible to the run.
    emit({
      kind: 'log',
      level: 'warn',
      message: `the server runs functions from main; an artifact uploaded to '${ctx.branch}' will not be found`,
    });
  }

  const details = await getFunctionDetails(ctx, target.node.name);
  const artifact = await getArtifactNode(ctx, location.workspace, location.nodePath);
  const plan = planRun({
    handlerOverridden: target.handlerOverridden,
    functionExists: details !== null,
    serverHash: artifact?.contentHash ?? null,
    localHash,
  });

  if (plan.mode === 'invoke') {
    emit({ kind: 'phase', message: `invoking the deployed function — ${plan.reason}` });
    const outcome = await invokeFunction(ctx, target.node.name, options.input, options.timeoutMs);
    for (const line of outcome.logs) emit({ kind: 'log', level: 'info', message: line });
    emit({ kind: 'result', outcome });
    return { outcome, plan };
  }

  emit({
    kind: 'phase',
    message: `uploading ${formatBytes(bytes.length)} to ${location.workspace}:/${location.nodePath} — ${plan.reason}`,
  });
  await uploadArtifact(
    ctx,
    location.workspace,
    location.nodePath,
    bytes,
    location.nodePath.split('/').pop() || 'main.wasm'
  );

  const uploaded = await getArtifactNode(ctx, location.workspace, location.nodePath);
  if (!uploaded?.id) {
    throw new Error(
      `uploaded ${location.workspace}:/${location.nodePath} but the server returned no node id for it`
    );
  }

  const outcome = await streamRunFile(
    ctx,
    {
      node_id: uploaded.id,
      handler: target.handler,
      input: options.input,
      timeout_ms: options.timeoutMs,
    },
    (frame) => {
      if (frame.event === 'log') {
        emit({
          kind: 'log',
          level: String(frame.data.level ?? 'info'),
          message: String(frame.data.message ?? ''),
        });
      } else if (frame.event === 'started') {
        emit({ kind: 'phase', message: `running (execution ${String(frame.data.execution_id)})` });
      }
    }
  );
  emit({ kind: 'result', outcome });
  return { outcome, plan };
}
