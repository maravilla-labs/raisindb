import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';

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
            <span>Open Source • Built with Rust</span>
          </div>
          <Heading as="h1" className={styles.heroTitle}>
            The Multi-Model Database
            <br />
            <span className={styles.heroTitleAccent}>With Git-like Workflows</span>
          </Heading>
          <p className={styles.heroSubtitle}>
            SQL queries, graph relationships, vector search, and full-text indexing.
            <br />
            Built for 2025 — repository-first, schema-driven, blazingly fast.
          </p>
          <div className={styles.ctaRow}>
            <Link className={clsx('button button--secondary button--lg', styles.ctaPrimary)} to="/docs/tutorials/quickstart">
              Get Started
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" className={styles.ctaIcon}>
                <path d="M6 3L11 8L6 13" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
              </svg>
            </Link>
            <Link className={clsx('button button--outline button--lg', styles.ctaSecondary)} to="/docs/access/rest/overview">
              View API Docs
            </Link>
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
            Why RaisinDB?
          </Heading>
          <p className={styles.sectionSubtitle}>
            A new approach to document storage that combines the flexibility of NoSQL with the power of version control.
          </p>
        </div>
        
        <div className={styles.bentoGrid}>
          <div className={clsx(styles.bentoCard, styles.bentoCardLarge)}>
            <div className={styles.bentoIcon}>🌳</div>
            <Heading as="h3" className={styles.bentoTitle}>Git-like Version Control</Heading>
            <p className={styles.bentoDesc}>
              Branches, tags, and commits for your documents. Merge workflows, audit trails, and time-travel queries built in.
            </p>
            <div className={styles.bentoCode}>
              <code>raisin branch create feature/new-content</code>
            </div>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}>📐</div>
            <Heading as="h3" className={styles.bentoTitle}>Schema Definitions</Heading>
            <p className={styles.bentoDesc}>
              Define NodeTypes with YAML. Strong typing, validation, inheritance, and constraints.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}>⚡</div>
            <Heading as="h3" className={styles.bentoTitle}>Built with Rust</Heading>
            <p className={styles.bentoDesc}>
              High performance, memory safety, and reliability. Powered by Axum and modern Rust async.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}>🔍</div>
            <Heading as="h3" className={styles.bentoTitle}>PostgreSQL-Compatible SQL</Heading>
            <p className={styles.bentoDesc}>
              Query with RaisinSQL - PostgreSQL dialect with hierarchical functions, JSON operators, and graph traversal.
            </p>
          </div>

          <div className={clsx(styles.bentoCard, styles.bentoCardWide)}>
            <div className={styles.bentoIcon}>🔗</div>
            <Heading as="h3" className={styles.bentoTitle}>Graph Database</Heading>
            <p className={styles.bentoDesc}>
              Navigate relationships with bidirectional edges. Query with NEIGHBORS() function for connected data.
            </p>
            <div className={styles.bentoCode}>
              <code>SELECT * FROM NEIGHBORS('user-123', 'OUT', 'AUTHORED')</code>
            </div>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}>🧠</div>
            <Heading as="h3" className={styles.bentoTitle}>Vector Similarity Search</Heading>
            <p className={styles.bentoDesc}>
              Semantic search with KNN() for embeddings. Perfect for AI applications and recommendations.
            </p>
          </div>

          <div className={styles.bentoCard}>
            <div className={styles.bentoIcon}>⚡</div>
            <Heading as="h3" className={styles.bentoTitle}>Full-Text Search</Heading>
            <p className={styles.bentoDesc}>
              Blazing-fast search powered by Tantivy. Multi-language stemming, fuzzy matching, and wildcards.
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
            Built for Modern Applications
          </Heading>
        </div>
        
        <div className={styles.useCaseGrid}>
          <div className={styles.useCase}>
            <div className={styles.useCaseNumber}>01</div>
            <Heading as="h3" className={styles.useCaseTitle}>Content Management</Heading>
            <p className={styles.useCaseDesc}>
              Manage articles, pages, and media with branching workflows. Perfect for editorial teams and multi-tenant CMS platforms.
            </p>
          </div>

          <div className={styles.useCase}>
            <div className={styles.useCaseNumber}>02</div>
            <Heading as="h3" className={styles.useCaseTitle}>Configuration Management</Heading>
            <p className={styles.useCaseDesc}>
              Version-controlled configs with rollback support. Deploy changes via branches and track history across environments.
            </p>
            <div className={styles.bentoCode} style={{marginTop: '1rem'}}>
              <code>raisin commit -m "Update prod config"</code>
            </div>
          </div>

          <div className={styles.useCase}>
            <div className={styles.useCaseNumber}>03</div>
            <Heading as="h3" className={styles.useCaseTitle}>Product Catalogs</Heading>
            <p className={styles.useCaseDesc}>
              Hierarchical product trees with schemas. Manage inventory, variants, and relationships with type safety.
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
            Ready to Get Started?
          </Heading>
          <p className={styles.ctaBoxDesc}>
            Install RaisinDB and start building version-controlled document systems today.
          </p>
          <div className={styles.ctaBoxButtons}>
            <Link className={clsx('button button--secondary button--lg')} to="/docs/getting-started/installation">
              View Installation Guide
            </Link>
            <a className={clsx('button button--outline button--lg')} href="https://github.com/maravilla-labs/raisindb" target="_blank" rel="noreferrer">
              Star on GitHub
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
