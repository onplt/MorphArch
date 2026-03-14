import type {ReactNode} from 'react';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';

import styles from './index.module.css';

type Capability = {
  title: string;
  body: string;
};

type Audience = {
  title: string;
  body: string;
};

type FAQ = {
  question: string;
  answer: string;
};

const capabilities: Capability[] = [
  {
    title: 'Scan Git history',
    body:
      'Walk first-parent history, store per-commit snapshots locally, and reuse cache state on repeated scans.',
  },
  {
    title: 'Build a dependency model',
    body:
      'Extract dependency edges from Rust, TypeScript, JavaScript, Python, and Go with safe fast paths and AST fallback.',
  },
  {
    title: 'Review structure, drift, and impact',
    body:
      'Open the map for grouped structure, inspect hotspots and score drivers, then check blast radius for risky modules.',
  },
  {
    title: 'Tune it per repository',
    body:
      'Use morpharch.toml to define ignore rules, package depth, boundaries, clustering, aliases, and presentation overrides.',
  },
];

const workflows: Capability[] = [
  {
    title: 'Map',
    body:
      'Start with grouped clusters and links so large repositories are readable before you look at individual nodes.',
  },
  {
    title: 'Cluster details',
    body:
      'Open one subsystem to inspect members, dependency pressure, and the most important incoming or outgoing links.',
  },
  {
    title: 'Inspect',
    body:
      'Focus one member when you need graph-level debugging instead of keeping the full raw graph on screen all the time.',
  },
];

const audiences: Audience[] = [
  {
    title: 'Architects',
    body:
      'Review boundaries, coupling, and structural pressure without manually reading a full dependency graph.',
  },
  {
    title: 'Tech leads',
    body:
      'Track drift over time, review what changed between commits, and identify parts of the repo that are becoming harder to change.',
  },
  {
    title: 'Developers',
    body:
      'Answer practical questions such as who depends on a module, what it pulls in, and what changes are likely to ripple outward.',
  },
];

const faqs: FAQ[] = [
  {
    question: 'What kind of repositories is MorphArch for?',
    answer:
      'It is most useful for large codebases where dependency structure and architectural drift are hard to inspect manually. It works well with monorepos, but it is also useful on smaller multi-package repositories.',
  },
  {
    question: 'Does it require a complex setup?',
    answer:
      'No. You can point it at a repository and start with defaults. Add morpharch.toml only when you want repo-specific control over ignore rules, scan heuristics, scoring, or clustering.',
  },
  {
    question: 'What languages are supported for dependency extraction?',
    answer:
      'Out of the box it supports Rust, TypeScript, JavaScript, Python, and Go.',
  },
];

function Hero({title}: {title: string}) {
  const demoSrc = useBaseUrl('/img/demo.gif');

  const handleCopy = () => {
    navigator.clipboard.writeText('cargo install morpharch');
  };

  return (
    <section className={styles.hero}>
      <div className="container">
        <div className={styles.heroGrid}>
          <div className={styles.heroCopy}>
            <div className={styles.eyebrow}>Open source terminal architecture analysis</div>
            <Heading as="h1" className={styles.heroTitle}>
              {title}
            </Heading>
            <p className={styles.heroSubtitle}>
              Scan Git history, build a dependency model, and inspect repository
              structure, drift, and hotspots from a terminal UI that stays usable
              on large codebases.
            </p>

            <div className={styles.heroActions}>
              <Link className="button button--primary button--lg" to="/docs/intro">
                Read the docs
              </Link>
              <Link
                className="button button--secondary button--lg"
                to="https://github.com/onplt/morpharch">
                View on GitHub
              </Link>
            </div>

            <div className={styles.installRow}>
              <code className={styles.installCode}>cargo install morpharch</code>
              <button className={styles.copyButton} onClick={handleCopy} type="button">
                Copy
              </button>
            </div>

            <div className={styles.metaRow}>
              <span className={styles.metaBadge}>First-parent history</span>
              <span className={styles.metaBadge}>Repo-scoped cache</span>
              <span className={styles.metaBadge}>Rust · TS · JS · Python · Go</span>
            </div>
          </div>

          <div className={styles.heroVisual}>
            <div className={styles.demoFrame}>
              <div className={styles.demoHeader}>
                <span className={styles.dotRed} />
                <span className={styles.dotYellow} />
                <span className={styles.dotGreen} />
                <span className={styles.demoTitle}>MorphArch TUI</span>
              </div>
              <img className={styles.demoImage} src={demoSrc} alt="MorphArch terminal UI demo" />
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function CompatibilityBar() {
  return (
    <section className={styles.compatibility}>
      <div className="container">
        <div className={styles.compatibilityRow}>
          <span className={styles.compatibilityLabel}>Works with</span>
          <span>Nx</span>
          <span>Turborepo</span>
          <span>pnpm</span>
          <span>Cargo</span>
          <span>Lerna</span>
        </div>
      </div>
    </section>
  );
}

function CapabilitySection() {
  return (
    <section className={styles.section}>
      <div className="container">
        <div className={styles.sectionIntro}>
          <Heading as="h2">What MorphArch helps you do</Heading>
          <p>
            MorphArch is designed for repeated local analysis. It is not just a
            graph renderer and it is not only a static score report.
          </p>
        </div>
        <div className={styles.cardGrid}>
          {capabilities.map(item => (
            <article key={item.title} className={styles.card}>
              <Heading as="h3">{item.title}</Heading>
              <p>{item.body}</p>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}

function WorkflowSection() {
  return (
    <section className={styles.sectionAlt}>
      <div className="container">
        <div className={styles.twoColumn}>
          <div>
            <div className={styles.sectionIntroLeft}>
              <Heading as="h2">See structure before raw graph detail</Heading>
              <p>
                Large repositories are easier to review when you start at the
                cluster level. MorphArch keeps the raw graph available, but only
                when you actually need it.
              </p>
            </div>
            <div className={styles.workflowList}>
              {workflows.map((item, index) => (
                <div key={item.title} className={styles.workflowItem}>
                  <div className={styles.workflowNumber}>{index + 1}</div>
                  <div>
                    <Heading as="h3">{item.title}</Heading>
                    <p>{item.body}</p>
                  </div>
                </div>
              ))}
            </div>
          </div>

          <aside className={styles.asidePanel}>
            <Heading as="h3">Built for repository review</Heading>
            <ul className={styles.checkList}>
              <li>Grouped map view instead of full raw graph by default</li>
              <li>Trend, hotspots, and blast radius in the same workflow</li>
              <li>Deterministic first-parent replay across history</li>
              <li>Configurable clustering, boundaries, and scan heuristics</li>
            </ul>
          </aside>
        </div>
      </div>
    </section>
  );
}

function AudienceSection() {
  return (
    <section className={styles.section}>
      <div className="container">
        <div className={styles.sectionIntro}>
          <Heading as="h2">Who it is for</Heading>
          <p>
            MorphArch is useful anywhere repository structure matters to daily
            engineering work.
          </p>
        </div>
        <div className={styles.cardGrid}>
          {audiences.map(item => (
            <article key={item.title} className={styles.card}>
              <Heading as="h3">{item.title}</Heading>
              <p>{item.body}</p>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}

function OpenSourceSection() {
  return (
    <section className={styles.sectionAlt}>
      <div className="container">
        <div className={styles.openSourcePanel}>
          <div>
            <Heading as="h2">Built in the open</Heading>
            <p>
              MorphArch is an open source project. Contributions around language
              support, clustering behavior, scan correctness, TUI polish, and
              documentation are welcome.
            </p>
          </div>
          <div className={styles.openSourceActions}>
            <Link className="button button--secondary button--lg" to="/docs/intro">
              Explore the docs
            </Link>
            <Link
              className="button button--primary button--lg"
              to="https://github.com/onplt/morpharch">
              GitHub repository
            </Link>
          </div>
        </div>
      </div>
    </section>
  );
}

function FAQSection() {
  return (
    <section className={styles.section}>
      <div className="container">
        <div className={styles.sectionIntro}>
          <Heading as="h2">Frequently asked questions</Heading>
        </div>
        <div className={styles.faqGrid}>
          {faqs.map(item => (
            <article key={item.question} className={styles.faqCard}>
              <Heading as="h3">{item.question}</Heading>
              <p>{item.answer}</p>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}

function FinalCTA() {
  return (
    <section className={styles.finalCta}>
      <div className="container">
        <div className={styles.finalCtaInner}>
          <Heading as="h2">Start with one scan</Heading>
          <p>
            Install MorphArch, scan a repository, and review structure and drift
            from the terminal.
          </p>
          <div className={styles.finalActions}>
            <code className={styles.installCode}>cargo install morpharch</code>
            <Link className="button button--primary button--lg" to="/docs/quick-start">
              Quick start
            </Link>
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
      title={`${siteConfig.title} | Repository structure and drift analysis`}
      description="MorphArch scans Git history, builds dependency graphs, and helps you inspect repository structure, drift, and hotspots from the terminal.">
      <main className={styles.page}>
        <Hero title={siteConfig.title} />
        <CompatibilityBar />
        <CapabilitySection />
        <WorkflowSection />
        <AudienceSection />
        <OpenSourceSection />
        <FAQSection />
        <FinalCTA />
      </main>
    </Layout>
  );
}
