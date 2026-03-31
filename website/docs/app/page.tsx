import Link from 'next/link';
import { GradientCard } from '@/components/GradientCard';
import { CodeSample } from '@/components/CodeSample';
import { coreHighlights, capabilityPillars } from '@/content/data/overview';
import { clientSample } from '@/content/data/clientSamples';

export default function HomePage() {
  return (
    <div className="space-y-16">
      <section className="section-shell mx-auto mt-6 max-w-6xl overflow-hidden">
        <div className="grid gap-10 px-8 py-12 md:grid-cols-2">
          <div className="space-y-6">
            <p className="text-xs uppercase tracking-[0.4em] text-raisin-200">RaisinDB</p>
            <h1 className="prose-title">
              Git-like, multi-model data for content graphs, documents, vectors, and realtime APIs.
            </h1>
            <div className="space-y-3 text-lg text-slate-300">
              <p>
                Branch and merge repositories like source code, define schemas with publishable NodeTypes, and query
                everything through the RaisinSQL engine that powers HTTP, WebSocket, and server-side rendering.
              </p>
              <ul className="space-y-2 text-sm text-slate-400">
                <li>• Hierarchical paths, versioned nodes, and workspace isolation.</li>
                <li>• SQL with path functions, JSON operators, vector distances, and Cypher bridges.</li>
                <li>• SDK parity across realtime WebSocket sessions and stateless HTTP workloads.</li>
              </ul>
            </div>
            <div className="flex flex-wrap gap-3">
              <Link
                href="/getting-started"
                className="rounded-full bg-white px-6 py-3 text-sm font-semibold text-slate-900 shadow-glow"
              >
                Start building
              </Link>
              <Link href="/multi-model" className="rounded-full border border-white/20 px-6 py-3 text-sm">
                Understand the model
              </Link>
            </div>
          </div>
          <CodeSample code={clientSample} title="Client SDK" caption="RaisinClient in action" />
        </div>
      </section>

      <section className="mx-auto max-w-6xl">
        <div className="grid gap-6 md:grid-cols-2">
          {coreHighlights.map((highlight) => (
            <GradientCard key={highlight.title} title={highlight.title} description={highlight.description} />
          ))}
        </div>
      </section>

      <section className="section-shell mx-auto max-w-6xl">
        <div className="flex flex-col gap-8">
          <div>
            <p className="text-xs uppercase tracking-[0.3em] text-raisin-200">Capability pillars</p>
            <h2 className="text-3xl font-semibold">Navigate from concepts into concrete APIs.</h2>
            <p className="mt-3 text-slate-300">
              Each pillar maps directly to the guides in this site—jump from ideas to SDK calls or REST endpoints in one
              click.
            </p>
          </div>
          <div className="grid gap-6 md:grid-cols-3">
            {capabilityPillars.map((pillar) => (
              <div key={pillar.name} className="rounded-3xl border border-white/10 bg-black/50 p-6">
                <p className="text-sm uppercase tracking-[0.3em] text-raisin-200">{pillar.name}</p>
                <ul className="mt-4 space-y-3 text-sm text-slate-300">
                  {pillar.items.map((item) => (
                    <li key={item} className="flex items-start gap-2">
                      <span className="mt-1 h-1.5 w-1.5 rounded-full bg-raisin-400" />
                      <span>{item}</span>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </div>
      </section>
    </div>
  );
}
