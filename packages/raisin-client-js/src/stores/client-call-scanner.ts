/**
 * Finds OPEN client-tool calls in an agent run's durable event log.
 *
 * The conversation SSE is a best-effort path for `client_tool_call`; the run's
 * event log is authoritative. A call is open when the relay parked on an
 * `external` request whose `resume_key` starts with `client:` and no later
 * `request_resolved` / `request_closed` names that request. Its name, args and
 * client id come from the `call_tool` effect (`args.client_call`) that the
 * relay was started with.
 */

import type { AgentRunEvent } from '../agent-runs';
import type { ClientToolCallEvent } from '../types/client-tools';

export const CLIENT_TOOL_RELAY = '/lib/raisin/ai/client-tool-relay';
const DEFAULT_TIMEOUT_MS = 120_000;

interface OpenRequest {
  resumeKey: string;
  expiresAtMs?: number;
}

export class ClientCallScanner {
  /** Highest seq ingested; pass to `events(run, lastSeq)`. */
  lastSeq = 0;
  terminal = false;
  private opened = new Map<string, OpenRequest>(); // request_id -> request
  private calls = new Map<string, Record<string, unknown>>(); // resume_key -> client_call
  private callSeq = new Map<string, number>(); // resume_key -> at_ms of the effect

  constructor(readonly runId: string) {}

  ingest(events: AgentRunEvent[]): void {
    for (const ev of events) {
      if (ev.seq > this.lastSeq) this.lastSeq = ev.seq;
      const kind = ev.kind as Record<string, any>;
      switch (kind.type) {
        case 'domain_applied':
          for (const effect of (kind.effects as unknown[]) ?? []) this.ingestEffect(effect, ev.at_ms);
          break;
        case 'request_opened': {
          const req = kind.request as Record<string, any> | undefined;
          const k = req?.kind as Record<string, any> | undefined;
          if (
            k?.kind === 'external' &&
            k.tool === CLIENT_TOOL_RELAY &&
            typeof k.resume_key === 'string' &&
            k.resume_key.startsWith('client:') &&
            typeof req?.request_id === 'string'
          ) {
            this.opened.set(req.request_id, {
              resumeKey: k.resume_key,
              expiresAtMs: typeof req.expires_at_ms === 'number' ? req.expires_at_ms : undefined,
            });
          }
          break;
        }
        case 'request_resolved':
        case 'request_closed':
          if (typeof kind.request_id === 'string') this.opened.delete(kind.request_id);
          break;
        case 'terminal':
          this.terminal = true;
          this.opened.clear();
          break;
      }
    }
  }

  private ingestEffect(effect: unknown, atMs: number): void {
    const e = effect as Record<string, any> | null;
    if (!e || e.kind !== 'call_tool' || e.tool !== CLIENT_TOOL_RELAY) return;
    const cc = e.args?.client_call as Record<string, unknown> | undefined;
    if (!cc || typeof cc.resume_key !== 'string') return;
    this.calls.set(cc.resume_key, cc);
    this.callSeq.set(cc.resume_key, atMs);
  }

  /** Calls still waiting for the client, as `client_tool_call` events. */
  openCalls(): ClientToolCallEvent[] {
    const out: ClientToolCallEvent[] = [];
    for (const req of this.opened.values()) {
      const cc = this.calls.get(req.resumeKey);
      if (!cc || typeof cc.call_id !== 'string' || typeof cc.name !== 'string' || typeof cc.client_id !== 'string') continue;
      const timeoutMs = typeof cc.timeout_ms === 'number' ? cc.timeout_ms : DEFAULT_TIMEOUT_MS;
      out.push({
        type: 'client_tool_call',
        run_id: this.runId,
        call_id: cc.call_id,
        resume_key: req.resumeKey,
        name: cc.name,
        args: (cc.args as Record<string, unknown>) ?? {},
        client_id: cc.client_id,
        expires_at_ms: req.expiresAtMs ?? (this.callSeq.get(req.resumeKey) ?? Date.now()) + timeoutMs,
      });
    }
    return out;
  }

  get hasOpen(): boolean {
    return this.opened.size > 0;
  }
}
