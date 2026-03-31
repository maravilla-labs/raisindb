import type {ReactNode} from 'react';
import clsx from 'clsx';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';
import { Database, GitBranch, Copy, Brain, Network, Search, FileCode, Globe, Radio, Plug, Zap, TreeDeciduous } from 'lucide-react';

import styles from './index.module.css';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className={styles.heroBackground} />
      <div className="container">
        <div className={styles.heroContent}>
          <div className={styles.badge}>
            <span className={styles.badgeDot} />
            <span>Coming Soon • Source Available (BSL 1.1) • Built with Rust</span>
          </div>
          <Heading as="h1" className={styles.heroTitle}>
            The Multi-Model Database
            <br />
            <span className={styles.heroTitleAccent}>With Git-like Workflows</span>
          </Heading>
          <p className={styles.heroSubtitle}>
            PostgreSQL-compatible SQL • OpenCypher compatible syntax • Vector embeddings • Full-text search • Real-time event streams
          </p>
          <div className={styles.comingSoonBox}>
            <div className={styles.comingSoonIcon}>🚀</div>
            <div className={styles.comingSoonText}>
              <strong>Coming Soon</strong>
              <p>Core features implemented: branching, revisions, copy/move operations, and hybrid search. Documentation and installation guides in progress.</p>
            </div>
          </div>
          <div className={styles.stats}>
            <div className={styles.stat}>
              <div className={styles.statValue}>BSL 1.1</div>
              <div className={styles.statLabel}>Open Source (2029)</div>
            </div>
            <div className={styles.statDivider} />
            <div className={styles.stat}>
              <div className={styles.statValue}>Git-like</div>
              <div className={styles.statLabel}>Branches, commits & tags</div>
            </div>
            <div className={styles.statDivider} />
            <div className={styles.stat}>
              <div className={styles.statValue}>Schema-driven</div>
              <div className={styles.statLabel}>Type-safe NodeTypes</div>
            </div>
          </div>
        </div>
      </div>
    </header>
  );
}

function FeatureGrid() {
  return (
    <section className={styles.featureSection}>
      <div className="container">
        <div className={styles.sectionHeader}>
          <Heading as="h2" className={styles.sectionTitle}>
            Core Features
          </Heading>
          <p className={styles.sectionSubtitle}>
            Multi-model database combining document storage, graph relationships, vector search, and full-text indexing with Git-inspired version control.
          </p>
        </div>

        <div className={styles.bentoGrid}>
          <div className={clsx(styles.bentoCard, styles.bentoCardLarge)}>
            <div className={styles.bentoIcon}><Database size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>PostgreSQL-Like SQL + Cypher DSL</Heading>
            <p className={styles.bentoDesc}>
              Query with familiar PostgreSQL syntax or use OpenCypher DSL for graph patterns. No need to learn entirely new languages.
            </p>
            <div className={styles.bentoCode}>
              <code>SELECT * FROM NEIGHBORS('node-id', 'OUT', 'AUTHORED')</code>
            </div>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><GitBranch size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Git-Like Branches & Tags</Heading>
            <p className={styles.bentoDesc}>
              Create branches for feature development, tag releases, query historical revisions, and merge changes with confidence.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><Copy size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Atomic Tree Operations</Heading>
            <p className={styles.bentoDesc}>
              Copy entire subtrees or move nodes atomically with copy_tree() and move_node(). Full revision tracking and diff support.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><Brain size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Vector Embeddings</Heading>
            <p className={styles.bentoDesc}>
              Generate embeddings natively or store your own. KNN() function for semantic similarity search in AI applications.
            </p>
          </div>

          <div className={clsx(styles.bentoCard, styles.bentoCardWide)}>
            <div className={styles.bentoIcon}><Network size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Graph Database</Heading>
            <p className={styles.bentoDesc}>
              Bidirectional edges with custom labels. Query with OpenCypher patterns or NEIGHBORS() in SQL for connected data.
            </p>
            <div className={styles.bentoCode}>
              <code>{`POST /edges — {"src": "node-1", "dst": "node-2", "label": "RELATES_TO"}`}</code>
            </div>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><Search size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Full-Text Search</Heading>
            <p className={styles.bentoDesc}>
              Tantivy-powered indexing with 20+ language support. Fuzzy matching, wildcards, and phrase queries.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><FileCode size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Schema Definitions</Heading>
            <p className={styles.bentoDesc}>
              YAML-based NodeType schemas with indexing control. Specify which properties are indexed for SQL or fulltext.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><TreeDeciduous size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Hierarchical Structure</Heading>
            <p className={styles.bentoDesc}>
              Organize data in tree structures with parent-child relationships. Efficient path-based queries and subtree operations.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><Radio size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Real-Time Events</Heading>
            <p className={styles.bentoDesc}>
              Subscribe to database changes with event-driven observability. React to node updates, commits, and schema changes.
            </p>
            <div className={styles.bentoCode}>
              <code>{`db.on("node:update", (event) => ...)`}</code>
            </div>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><Plug size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>Native SDKs</Heading>
            <p className={styles.bentoDesc}>
              Connect from Node.js, Browser, Rust, or Java with official type-safe SDKs and full API coverage.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}><Zap size={40} strokeWidth={1.5} /></div>
            <Heading as="h3" className={styles.bentoTitle}>RocksDB Backend</Heading>
            <p className={styles.bentoDesc}>
              Built with Rust on RocksDB. Atomic transactions, efficient key-value storage, and LSM-tree architecture.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function UseCases() {
  return (
    <section className={styles.useCaseSection}>
      <div className="container">
        <div className={styles.sectionHeader}>
          <Heading as="h2" className={styles.sectionTitle}>
            Technical Architecture
          </Heading>
        </div>

        <div className={styles.useCaseGrid}>
          <div className={styles.useCase}>
            <div className={styles.useCaseNumber}>01</div>
            <Heading as="h3" className={styles.useCaseTitle}>Hierarchical + Graph + Vector</Heading>
            <p className={styles.useCaseDesc}>
              Combine tree structures for document hierarchies, graph edges for relationships, and vector embeddings for semantic search — all in one database.
            </p>
          </div>

          <div className={styles.useCase}>
            <div className={styles.useCaseNumber}>02</div>
            <Heading as="h3" className={styles.useCaseTitle}>Familiar Query Languages</Heading>
            <p className={styles.useCaseDesc}>
              Use PostgreSQL-compatible SQL for relational queries and OpenCypher DSL for graph patterns. Leverage your existing knowledge without learning proprietary query languages.
            </p>
          </div>

          <div className={styles.useCase}>
            <div className={styles.useCaseNumber}>03</div>
            <Heading as="h3" className={styles.useCaseTitle}>Version Control Built-In</Heading>
            <p className={styles.useCaseDesc}>
              Every repository has Git-like branching, tagging, and merge capabilities. Test changes in isolation, rollback instantly, and maintain complete audit trails for compliance.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function CTASection() {
  return (
    <section className={styles.ctaSection}>
      <div className="container">
        <div className={styles.ctaBox}>
          <Heading as="h2" className={styles.ctaBoxTitle}>
            Stay Connected
          </Heading>
          <p className={styles.ctaBoxDesc}>
            Follow our progress on GitHub and be the first to know when RaisinDB launches.
          </p>
          <div className={styles.ctaBoxButtons}>
            <a className={clsx('button button--secondary button--lg')} href="https://github.com/maravilla-labs/raisindb" target="_blank" rel="noreferrer">
              Star on GitHub
            </a>
            <a className={clsx('button button--outline button--lg')} href="https://github.com/maravilla-labs/raisindb/issues" target="_blank" rel="noreferrer">
              Report Issues
            </a>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function Home(): ReactNode {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title={`${siteConfig.title} - Multi-Model Database with Git-like Versioning`}
      description="RaisinDB is an open-source multi-model database with SQL queries, graph relationships, vector search, and full-text indexing. Features git-like versioning with branches, commits, and schema definitions. Built for modern applications.">
      <HomepageHeader />
      <main>
        <FeatureGrid />
        <UseCases />
        <CTASection />
      </main>
    </Layout>
  );
}
