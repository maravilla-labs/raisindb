import { PageShell } from '@/components/PageShell';
import { CodeSample } from '@/components/CodeSample';
import { sqlTemplateExample, sqlHttpExample } from '@/content/data/sqlExamples';
import { sqlPlaybook } from '@/content/data/sqlPlaybook';

const engineHighlights = [
  'The RaisinSQL engine runs a full pipeline: AST parsing, semantic analysis, logical planning, optimization, and physical execution (as exposed in the `raisin_sql` crate).',
  'Hierarchy helpers (`PATH_STARTS_WITH`, `PARENT`, `DEPTH`, `ANCESTOR`) are native functions, so filtering trees is index friendly.',
  'JSON operators, vector distance symbols (<->, <=>, <#>), Tantivy full-text search, and Cypher bridges all run under the same execution context.',
  'EXPLAIN and EXPLAIN (VERBOSE) surface each stage of the plan so you can inspect optimizations before shipping queries.',
];

export default function SqlPage() {
  return (
    <div className="space-y-10">
      <PageShell
        eyebrow="Query"
        title="SQL over RaisinDB"
        subtitle="One engine powers template literals, REST calls, full-text search, vector KNN, and graph queries."
      >
        <div className="grid gap-6 md:grid-cols-2">
          <CodeSample code={sqlTemplateExample} caption="Template literal" />
          <CodeSample code={sqlHttpExample} language="json" caption="HTTP payload" />
        </div>

        <div className="grid gap-6 lg:grid-cols-3">
          {sqlPlaybook.map((entry) => (
            <div key={entry.title} className="space-y-3 rounded-3xl border border-white/10 bg-black/60 p-6">
              <h3 className="text-lg font-semibold text-white">{entry.title}</h3>
              <p className="text-sm text-slate-400">{entry.description}</p>
              <CodeSample code={entry.snippet} language="sql" caption="Example" />
            </div>
          ))}
        </div>

        <div className="rounded-3xl border border-white/10 bg-black/60 p-6">
          <h3 className="text-2xl font-semibold">Engine highlights</h3>
          <ul className="mt-4 space-y-3 text-sm text-slate-300">
            {engineHighlights.map((highlight) => (
              <li key={highlight} className="flex items-start gap-2">
                <span className="mt-2 h-1.5 w-1.5 rounded-full bg-raisin-400" />
                <span>{highlight}</span>
              </li>
            ))}
          </ul>
        </div>
      </PageShell>
    </div>
  );
}
