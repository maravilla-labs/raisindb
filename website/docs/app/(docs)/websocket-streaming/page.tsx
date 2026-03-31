import { PageShell } from '@/components/PageShell';
import { CodeSample } from '@/components/CodeSample';
import { clientSample } from '@/content/data/clientSamples';
import { realtimeSteps } from '@/content/data/realtime';

const streamingNotes = [
  'Messages are MessagePack-encoded envelopes (`RequestEnvelope`, `ResponseEnvelope`, and event variants) so payloads stay compact over the wire.',
  'Request types map to the `RequestType` enum, ensuring the server can safely route database, SQL, and schema operations over one socket.',
  'Streaming responses use `ResponseStatus.Streaming`, so large SQL queries or tree reads emit incremental chunks without closing the channel.',
];

export default function WebsocketStreamingPage() {
  return (
    <div className="space-y-10">
      <PageShell
        eyebrow="Realtime"
        title="WebSocket transport"
        subtitle="Low-latency operations with automatic request tracking, MessagePack envelopes, and workspace-scoped events."
      >
        <CodeSample code={clientSample} caption="Session bootstrap" />
        <div className="grid gap-6 md:grid-cols-3">
          {realtimeSteps.map((step) => (
            <div key={step.title} className="rounded-3xl border border-white/10 bg-white/5 p-6">
              <p className="text-sm font-semibold text-white">{step.title}</p>
              <p className="mt-2 text-sm text-slate-300">{step.detail}</p>
            </div>
          ))}
        </div>
        <div className="rounded-3xl border border-white/10 bg-black/60 p-6">
          <h3 className="text-2xl font-semibold">Protocol details</h3>
          <ul className="mt-4 space-y-3 text-sm text-slate-300">
            {streamingNotes.map((note) => (
              <li key={note} className="flex items-start gap-2">
                <span className="mt-2 h-1.5 w-1.5 rounded-full bg-raisin-400" />
                <span>{note}</span>
              </li>
            ))}
          </ul>
        </div>
      </PageShell>
    </div>
  );
}
