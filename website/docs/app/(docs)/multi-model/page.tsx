import { PageShell } from '@/components/PageShell';
import { GradientCard } from '@/components/GradientCard';
import { CodeSample } from '@/components/CodeSample';
import { dataModelAspects } from '@/content/data/dataModel';
import { nodeTypeYaml, archetypeYaml, elementTypeYaml, modelingSdkSnippet } from '@/content/data/modelingSamples';

const governance = [
  'Schema APIs live under `/api/management/{repo}/{branch}` and match the SDK helpers (`nodeTypes()`, `archetypes()`, `elementTypes()`).',
  'Branches are first-class: call `database.onBranch()` or pass `?branch=` to HTTP to stage schema and content safely.',
  'Translation, embeddings, and tags are repository-level settings so you can audit every change alongside content revisions.',
];

export default function MultiModelPage() {
  return (
    <div className="space-y-10">
      <PageShell
        eyebrow="Data model"
        title="Multi-model building blocks"
        subtitle="RaisinDB stores hierarchical nodes with publishable schemas, Git-like branches, and reusable presentation layers."
      >
        <div className="grid gap-6 md:grid-cols-2">
          {dataModelAspects.map((aspect) => (
            <GradientCard key={aspect.name} title={aspect.name} description={aspect.description} />
          ))}
        </div>

        <div className="grid gap-6 lg:grid-cols-3">
          <div className="space-y-3 rounded-3xl border border-white/10 bg-black/60 p-6">
            <h3 className="text-lg font-semibold text-white">NodeType (YAML)</h3>
            <p className="text-sm text-slate-400">
              Defines allowed properties, child relationships, and indexing behavior. The fields map 1:1 to the Rust
              `NodeType` struct, so validation happens before anything is persisted.
            </p>
            <CodeSample code={nodeTypeYaml} language="yaml" caption="Schema + constraints" />
          </div>
          <div className="space-y-3 rounded-3xl border border-white/10 bg-black/60 p-6">
            <h3 className="text-lg font-semibold text-white">Archetype (YAML)</h3>
            <p className="text-sm text-slate-400">
              Archetypes extend NodeTypes with pre-configured fields, starter content, and optional inheritance chains for
              editors.
            </p>
            <CodeSample code={archetypeYaml} language="yaml" caption="Authoring presets" />
          </div>
          <div className="space-y-3 rounded-3xl border border-white/10 bg-black/60 p-6">
            <h3 className="text-lg font-semibold text-white">ElementType (YAML)</h3>
            <p className="text-sm text-slate-400">
              ElementTypes describe reusable fragments (layout, view config, field schema) that can be embedded inside
              node properties.
            </p>
            <CodeSample code={elementTypeYaml} language="yaml" caption="Reusable fragments" />
          </div>
        </div>

        <div className="rounded-3xl border border-white/10 bg-black/60 p-6">
          <h3 className="text-2xl font-semibold">Manage schemas through the SDK</h3>
          <p className="mt-3 text-slate-300">
            Database handles expose strongly typed helpers for every governance action: create/update, publish,
            validate, and branch.
          </p>
          <CodeSample code={modelingSdkSnippet} language="typescript" caption="Schema workflows" />
        </div>

        <div className="rounded-3xl border border-white/10 bg-black/60 p-6">
          <h3 className="text-2xl font-semibold">Governance + publishing</h3>
          <ul className="mt-4 space-y-3 text-sm text-slate-300">
            {governance.map((item) => (
              <li key={item} className="flex items-start gap-2">
                <span className="mt-2 h-1.5 w-1.5 rounded-full bg-raisin-400" />
                <span>{item}</span>
              </li>
            ))}
          </ul>
        </div>
      </PageShell>
    </div>
  );
}
