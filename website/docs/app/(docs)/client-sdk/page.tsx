import { PageShell } from '@/components/PageShell';
import { GradientCard } from '@/components/GradientCard';
import { CodeSample } from '@/components/CodeSample';
import { clientSample, httpSample } from '@/content/data/clientSamples';

const sdkFacts = [
  {
    title: 'Context-aware routing',
    description:
      'Calling `client.database(name)` stamps tenant, repository, branch, and revision into every downstream SQL or workspace request, mirroring the Git-like execution context exposed in the transports.',
  },
  {
    title: 'Workspace ergonomics',
    description:
      'Workspace clients lazily provide node helpers, event subscriptions, and transactions so you can mutate trees, listen to events, and roll back work without re-building payloads.',
  },
  {
    title: 'HTTP parity',
    description:
      '`RaisinClient.forSSR` returns the HTTP client with the exact same surface area, so loaders, CLI tools, and background jobs run the identical APIs without a WebSocket.',
  },
];

const nodeOps = [
  'Create, update, delete, and query nodes with strongly typed payloads (`NodeCreatePayload`, `NodeQueryPayload`).',
  'Tree helpers: list children, hydrate trees, move/rename/copy nodes, reorder siblings.',
  'Graph helpers: add/remove relations and inspect incoming/outgoing edges with workspace-aware targeting.',
];

export default function ClientSdkPage() {
  return (
    <div className="space-y-10">
      <PageShell
        eyebrow="Client SDK"
        title="@raisin-client-js"
        subtitle="One set of helpers powers WebSocket sessions, HTTP fallbacks, and hybrid rendering."
      >
        <div className="grid gap-6 md:grid-cols-2">
          <CodeSample code={clientSample} caption="RaisinClient" />
          <CodeSample code={httpSample} caption="RaisinHttpClient" />
        </div>
        <div className="grid gap-6 md:grid-cols-3">
          {sdkFacts.map((fact) => (
            <GradientCard key={fact.title} title={fact.title} description={fact.description} />
          ))}
        </div>
        <div className="rounded-3xl border border-white/10 bg-black/60 p-6">
          <h3 className="text-2xl font-semibold">Node + relation APIs</h3>
          <p className="mt-3 text-slate-300">Available on every workspace client:</p>
          <ul className="mt-5 space-y-3 text-sm text-slate-200">
            {nodeOps.map((item) => (
              <li key={item} className="flex items-start gap-2">
                <span className="mt-2 h-1.5 w-1.5 rounded-full bg-raisin-400" />
                <span>{item}</span>
              </li>
            ))}
          </ul>
        </div>
        <div className="grid gap-6 md:grid-cols-2">
          <div className="rounded-3xl border border-white/10 bg-white/5 p-6">
            <h4 className="text-lg font-semibold">Transactions</h4>
            <p className="mt-2 text-sm text-slate-300">
              `workspace.transaction()` exposes begin/commit/rollback while reusing the same request context, so multi-step node mutations stay atomic per branch.
            </p>
          </div>
          <div className="rounded-3xl border border-white/10 bg-white/5 p-6">
            <h4 className="text-lg font-semibold">Events</h4>
            <p className="mt-2 text-sm text-slate-300">
              The event layer multiplexes MessagePack frames into workspace-specific channels. Use
              <code className="mx-1 font-mono text-xs text-slate-100">
                workspace(name).events().on(&apos;node.created&apos;, handler)
              </code>
              to react to repository changes without polling.
            </p>
          </div>
        </div>
      </PageShell>
    </div>
  );
}
