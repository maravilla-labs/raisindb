import Link from 'next/link';
import { PageShell } from '@/components/PageShell';
import { CodeSample } from '@/components/CodeSample';
import { onboarding } from '@/content/data/gettingStarted';
import { clientSample, httpSample } from '@/content/data/clientSamples';

export default function GettingStartedPage() {
  return (
    <div className="space-y-10">
      <PageShell
        eyebrow="Workflow"
        title="Getting started"
        subtitle="Connect, authenticate, and start modeling data through RaisinClient. Pick WebSocket for realtime UX or the HTTP client for build steps and SSR."
        actions={
          <Link href="/client-sdk" className="rounded-full bg-white px-5 py-2 text-sm font-semibold text-slate-900">
            Dive into the SDK
          </Link>
        }
      >
        <div className="grid gap-6 md:grid-cols-2">
          <CodeSample code={clientSample} caption="WebSocket mode" />
          <CodeSample code={httpSample} caption="HTTP-only mode" />
        </div>
        <div className="grid gap-5 md:grid-cols-2">
          {onboarding.map((item) => (
            <div key={item.step} className="rounded-3xl border border-white/10 bg-white/5 p-6">
              <p className="text-xs uppercase tracking-[0.4em] text-raisin-200">{item.step}</p>
              <p className="mt-2 text-base text-slate-200">{item.detail}</p>
            </div>
          ))}
        </div>
        <div className="rounded-3xl border border-white/10 bg-gradient-to-br from-raisin-500/10 to-transparent p-6 text-sm text-slate-300">
          <p className="font-semibold text-white">Environment requirements</p>
          <ul className="mt-3 space-y-2">
            <li>WebSocket URLs follow the `raisin://tenant/repository` or `ws(s)` formats consumed by `RaisinClient`.</li>
            <li>HTTP URLs target the `/api/*` surface shown on the REST page; both transports share the same auth model.</li>
            <li>Authentication is mandatory for any repository or workspace operations.</li>
          </ul>
        </div>
      </PageShell>
    </div>
  );
}
