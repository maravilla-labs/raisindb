import { Link, useParams } from 'react-router-dom'
import { Layers, Tag, Puzzle, Shapes, GitBranch, Network } from 'lucide-react'

export default function Models() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'

  const scoped = (path: string) =>
    repo ? `/${repo}/${branch ? `${activeBranch}/` : ''}${path}` : '#'

  return (
    <div className="max-w-5xl mx-auto px-6 py-12 text-white space-y-12">
      <header className="space-y-4">
        <div className="flex items-center gap-3 text-primary-300">
          <Layers className="w-8 h-8" />
          <h1 className="text-3xl font-bold">Models Overview</h1>
        </div>
        <p className="text-white/70 leading-relaxed">
          RaisinDB is a hierarchical, multi-model, git-like database. Branches capture the full
          history of every schema decision, workspaces provide isolated trees, and nodes represent
          arbitrary domain objects—from devices and datasets to knowledge graph entities and
          documents. Models are where you decide how those nodes behave. Use this hub to introduce a{' '}
          <strong>Node Type</strong>, layer specialised behaviour with an <strong>Archetype</strong>,
          or define reusable <strong>Element Types</strong> for complex composite properties.
        </p>
      </header>

      <section className="grid grid-cols-1 md:grid-cols-3 gap-6">
        <div className="bg-white/5 border border-white/10 rounded-2xl p-6 space-y-3">
          <h2 className="flex items-center gap-2 text-lg font-semibold text-white">
            <Tag className="w-5 h-5 text-primary-300" />
            Node Types
          </h2>
          <p className="text-sm text-white/70 leading-relaxed">
            Define structural rules for nodes: property schemas, inheritance, allowed children, and
            indexing preferences. Node types propagate down the tree so you can govern any domain at
            scale—asset hierarchies, digital twins, regulatory dossiers, knowledge graphs, and more.
          </p>
          <ul className="text-sm text-white/60 space-y-2 list-disc list-inside">
            <li>Supports 15+ property value kinds including references, resources, elements, and composites for structured metadata.</li>
            <li>Git-like revisions with branch-aware change tracking, reviews, and publish workflow.</li>
            <li>Controls workspace scaffolding, search indexing (full-text, vector, property), and child constraints across the tree.</li>
          </ul>
          <Link
            to={scoped('nodetypes')}
            className="inline-flex items-center gap-2 text-primary-300 hover:text-primary-200 text-sm font-semibold"
          >
            Manage node types →
          </Link>
        </div>

        <div className="bg-white/5 border border-white/10 rounded-2xl p-6 space-y-3">
          <h2 className="flex items-center gap-2 text-lg font-semibold text-white">
            <Puzzle className="w-5 h-5 text-primary-300" />
            Archetypes
          </h2>
          <p className="text-sm text-white/70 leading-relaxed">
            Specialised presets layered on top of a node type. Archetypes capture opinionated field
            sets, default values, and workflow hints without forking the underlying schema—ideal for
            product variants, release bundles, scenario modelling, or jurisdiction-specific rules.
          </p>
          <ul className="text-sm text-white/60 space-y-2 list-disc list-inside">
            <li>Bind to a base node type while adding additional fields, initial state, or orchestration metadata.</li>
            <li>Great for domain presets like manufacturing bill-of-process variants, compliance playbooks, or deployment profiles.</li>
            <li>Versioned and branch-aware, so archetype updates follow the same review and merge flow as schemas.</li>
          </ul>
          <Link
            to={scoped('archetypes')}
            className="inline-flex items-center gap-2 text-primary-300 hover:text-primary-200 text-sm font-semibold"
          >
            Browse archetypes →
          </Link>
        </div>

        <div className="bg-white/5 border border-white/10 rounded-2xl p-6 space-y-3">
          <h2 className="flex items-center gap-2 text-lg font-semibold text-white">
            <Shapes className="w-5 h-5 text-primary-300" />
            Element Types
          </h2>
          <p className="text-sm text-white/70 leading-relaxed">
            Define reusable composites for nested structures—think sensor payload schemas, pricing
            bands, checklist steps, or knowledge snippets. Elements keep complex embedded data
            consistent across nodes.
          </p>
          <ul className="text-sm text-white/60 space-y-2 list-disc list-inside">
            <li>Composable fields with nested sections, calculated values, references, or other composites.</li>
            <li>Ideal for time-series datapoints, incident timelines, multi-step procedures, or machine configuration matrices.</li>
            <li>Each element type can ship with optional initial data, validation rules, and view metadata for consumers.</li>
          </ul>
          <Link
            to={scoped('elementtypes')}
            className="inline-flex items-center gap-2 text-primary-300 hover:text-primary-200 text-sm font-semibold"
          >
            Manage element types →
          </Link>
        </div>
      </section>

      <section className="bg-white/5 border border-white/10 rounded-2xl p-8 space-y-6">
        <h2 className="text-xl font-semibold text-white flex items-center gap-3">
          <Network className="w-5 h-5 text-primary-300" />
          How the model layer works together
        </h2>
        <div className="grid md:grid-cols-2 gap-6 text-sm text-white/70 leading-relaxed">
          <div className="space-y-3">
            <p>
              Start with a Node Type to define the canonical structure of an area in your graph. Node
              types participate in hierarchy, versioning, access control, and indexing. They can
              inherit, mix in traits, and declare allowed child types so large domain models stay
              predictable even as teams branch, simulate, and merge.
            </p>
            <p>
              Add Archetypes when teams need repeatable blueprints—fleet onboarding packets, lab
              experiment templates, regulatory filings, or rollout playbooks. Archetypes leave the
              base schema untouched while providing curated fields, default data, or workflow
              markers.
            </p>
          </div>
          <div className="space-y-3">
            <p>
              Element Types describe reusable composites. Composite properties on a node type (or
              fields contributed by an archetype) can reference element types, letting you assemble
              structured payloads from verified fragments. Elements bring their own validation,
              defaults, and presentation hints so downstream systems can render or process them
              safely.
            </p>
            <p>
              Need inspiration or baseline definitions? Explore the existing catalog in the{' '}
              <Link
                to={scoped('elementtypes')}
                className="text-primary-300 hover:text-primary-200 font-semibold"
              >
                Element Types library
              </Link>{' '}
              or pull down the YAML definitions directly from Git to review diffs before merging.
            </p>
          </div>
        </div>
      </section>

      <section className="bg-white/5 border border-white/10 rounded-2xl p-8 space-y-4">
        <h2 className="text-xl font-semibold text-white flex items-center gap-3">
          <GitBranch className="w-5 h-5 text-primary-300" />
          Helpful resources
        </h2>
        <ul className="text-white/70 text-sm space-y-3 list-disc list-inside leading-relaxed">
          <li>
            <a
              href="https://docs.raisindb.com/nodetypes/property-types"
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary-300 hover:text-primary-200 font-semibold"
            >
              Property reference
            </a>{' '}
            — deep dive into every property value type, including Element and Composite fields.
          </li>
          <li>
            <a
              href="https://docs.raisindb.com/architecture/models"
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary-300 hover:text-primary-200 font-semibold"
            >
              Modeling patterns
            </a>{' '}
            — best practices for composing node types, archetypes, and elements across branches.
          </li>
          <li>
            Review your current branch (<span className="text-white">{activeBranch}</span>) and
            compare YAML changes in Git to keep schema migrations auditable and merge-friendly.
          </li>
        </ul>
      </section>
    </div>
  )
}
