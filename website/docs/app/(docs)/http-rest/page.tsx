import { PageShell } from '@/components/PageShell';
import { CodeSample } from '@/components/CodeSample';
import { httpSample } from '@/content/data/clientSamples';
import { httpRoutes } from '@/content/data/httpRoutes';
import { sqlHttpExample } from '@/content/data/sqlExamples';

export default function HttpRestPage() {
  return (
    <div className="space-y-10">
      <PageShell
        eyebrow="Transport"
        title="HTTP & REST endpoints"
        subtitle="Deterministic, stateless endpoints for automation, loaders, and server components."
      >
        <CodeSample code={httpSample} caption="Client helper" />
        <CodeSample code={sqlHttpExample} language="json" caption="Direct POST to /api/sql/{repo}" />
        <div className="space-y-8">
          {httpRoutes.map((group) => (
            <div key={group.group} className="rounded-3xl border border-white/10 bg-black/60 p-6">
              <div className="flex flex-col gap-1">
                <p className="text-xs uppercase tracking-[0.4em] text-raisin-200">{group.group}</p>
                <p className="text-sm text-slate-400">{group.description}</p>
              </div>
              <div className="mt-4 overflow-hidden rounded-2xl border border-white/5">
                <table className="w-full text-left text-sm text-slate-200">
                  <thead className="bg-white/5 text-xs uppercase tracking-widest text-slate-400">
                    <tr>
                      <th className="px-4 py-3">Method</th>
                      <th className="px-4 py-3">Path</th>
                      <th className="px-4 py-3">Notes</th>
                    </tr>
                  </thead>
                  <tbody>
                    {group.routes.map((route) => (
                      <tr key={`${route.method}-${route.path}`} className="border-t border-white/5">
                        <td className="px-4 py-3 font-semibold text-raisin-200">{route.method}</td>
                        <td className="px-4 py-3 font-mono text-xs">{route.path}</td>
                        <td className="px-4 py-3 text-slate-300">{route.notes}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          ))}
        </div>
      </PageShell>
    </div>
  );
}
