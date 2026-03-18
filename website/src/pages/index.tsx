import type {ReactNode} from 'react';
import {useState, useEffect, useRef} from 'react';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';

import styles from './index.module.css';

/* ─── Data ─────────────────────────────────────────────────────────────────── */

type Feature = {
  icon: string;
  title: string;
  body: string;
};

type ScoreComponent = {
  name: string;
  weight: string;
  color: string;
  description: string;
};

type Step = {
  number: string;
  title: string;
  body: string;
};

const features: Feature[] = [
  {
    icon: '⟳',
    title: 'Git-native history scanning',
    body: 'Walk first-parent history with gix (pure Rust). Store per-commit snapshots locally and reuse cache on repeated scans.',
  },
  {
    icon: '⟡',
    title: 'Language-aware parsing',
    body: 'Extract dependency edges from Rust, TypeScript, JavaScript, Python, and Go with comment-safe fast paths and tree-sitter AST fallback.',
  },
  {
    icon: '◈',
    title: 'Scale-aware health scoring',
    body: 'Six-component 0–100 health score that adapts baselines for repository size. No false positives on large monorepos.',
  },
  {
    icon: '◎',
    title: 'Blast radius analysis',
    body: 'Inspect downstream impact for high-risk modules. See which changes are likely to ripple through the dependency graph.',
  },
  {
    icon: '▦',
    title: 'Cluster-first navigation',
    body: 'Start with a grouped map view. Drill into cluster details and then individual modules — never start with raw graph chaos.',
  },
  {
    icon: '⬡',
    title: 'AI architecture assistant',
    body: 'Ask natural language questions about health, coupling, hotspots, and blast radius. Works with OpenAI, Ollama, or any compatible API.',
  },
];

const scoreComponents: ScoreComponent[] = [
  {name: 'Cycle', weight: '30%', color: '#f7768e', description: 'Circular dependency detection via SCC analysis'},
  {name: 'Layering', weight: '25%', color: '#ff9e64', description: 'Back-edges violating layered architecture'},
  {name: 'Hub', weight: '15%', color: '#e0af68', description: 'God modules with high fan-in and fan-out'},
  {name: 'Coupling', weight: '12%', color: '#9ece6a', description: 'Edge weight density and import concentration'},
  {name: 'Cognitive', weight: '10%', color: '#7dcfff', description: 'Graph complexity and degree excess'},
  {name: 'Instability', weight: '8%', color: '#bb9af7', description: 'Brittle modules via Martin instability metric'},
];

const workflowSteps: Step[] = [
  {
    number: '01',
    title: 'Map',
    body: 'Start with grouped clusters so large repositories are readable before looking at individual nodes.',
  },
  {
    number: '02',
    title: 'Cluster details',
    body: 'Open a subsystem to inspect members, dependencies, and link pressure between clusters.',
  },
  {
    number: '03',
    title: 'Inspect',
    body: 'Focus one module with a one-hop subgraph. See who depends on it, what it pulls in, and its blast radius.',
  },
];

const installMethods = [
  {label: 'Cargo', command: 'cargo install morpharch'},
  {label: 'Homebrew', command: 'brew install onplt/morpharch'},
  {label: 'npm', command: 'npm install -g morpharch'},
  {label: 'Shell', command: 'curl -fsSL https://raw.githubusercontent.com/onplt/morpharch/main/install.sh | sh'},
];

/* ─── Components ───────────────────────────────────────────────────────────── */

function useInView(threshold = 0.15) {
  const ref = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    // SSR safety
    if (typeof window === 'undefined' || !('IntersectionObserver' in window)) {
      setVisible(true);
      return;
    }

    const el = ref.current;
    if (!el) return;

    const obs = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          obs.disconnect();
        }
      },
      {threshold},
    );
    obs.observe(el);
    return () => obs.disconnect();
  }, [threshold]);

  return {ref, visible};
}

function Reveal({children, delay = 0}: {children: ReactNode; delay?: number}) {
  const {ref, visible} = useInView();
  return (
    <div
      ref={ref}
      className={`${styles.reveal} ${visible ? styles.revealVisible : ''}`}
      style={{transitionDelay: `${delay}ms`}}>
      {children}
    </div>
  );
}

function Hero() {
  const demoSrc = useBaseUrl('/img/demo.gif');
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText('cargo install morpharch');
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <section className={styles.hero}>
      <div className={styles.heroGlow} />
      <div className="container">
        <div className={styles.heroGrid}>
          <div className={styles.heroCopy}>
            <div className={styles.eyebrow}>
              <span className={styles.eyebrowDot} />
              Open-source architecture analysis
            </div>

            <Heading as="h1" className={styles.heroTitle}>
              See the structure
              <br />
              your repo hides
            </Heading>

            <p className={styles.heroSubtitle}>
              MorphArch scans Git history, builds dependency graphs, computes
              health scores, and helps you inspect structure, drift, and
              hotspots — all from a terminal UI designed for large codebases.
            </p>

            <div className={styles.heroActions}>
              <Link className={styles.btnPrimary} to="/docs/quick-start">
                Get started
              </Link>
              <Link className={styles.btnGhost} to="https://github.com/onplt/morpharch">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z" />
                </svg>
                GitHub
              </Link>
            </div>

            <div className={styles.installRow}>
              <code className={styles.installCode}>
                <span className={styles.installPrompt}>$</span> cargo install morpharch
              </code>
              <button
                className={styles.copyButton}
                onClick={handleCopy}
                type="button"
                aria-label="Copy install command">
                {copied ? '✓ Copied' : 'Copy'}
              </button>
            </div>

            <div className={styles.langRow}>
              {['Rust', 'TypeScript', 'JavaScript', 'Python', 'Go'].map(lang => (
                <span key={lang} className={styles.langBadge}>{lang}</span>
              ))}
            </div>
          </div>

          <div className={styles.heroVisual}>
            <div className={styles.demoFrame}>
              <div className={styles.demoHeader}>
                <span className={styles.dotRed} />
                <span className={styles.dotYellow} />
                <span className={styles.dotGreen} />
                <span className={styles.demoTitle}>morpharch watch .</span>
              </div>
              <img
                className={styles.demoImage}
                src={demoSrc}
                alt="MorphArch terminal UI showing architecture health dashboard"
                loading="eager"
              />
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function CompatibilityBar() {
  const tools = ['Nx', 'Turborepo', 'pnpm workspaces', 'Cargo workspaces', 'Lerna'];
  return (
    <section className={styles.compatibility}>
      <div className="container">
        <div className={styles.compatRow}>
          <span className={styles.compatLabel}>Works with</span>
          <div className={styles.compatLogos}>
            {tools.map(tool => (
              <span key={tool} className={styles.compatItem}>{tool}</span>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}

function FeatureSection() {
  return (
    <section className={styles.section} id="features">
      <div className="container">
        <Reveal>
          <div className={styles.sectionHeader}>
            <span className={styles.sectionLabel}>Capabilities</span>
            <Heading as="h2">Everything you need to understand your architecture</Heading>
            <p>
              MorphArch is designed for repeated local analysis — not a one-time
              graph dump, but an operational tool for ongoing architecture review.
            </p>
          </div>
        </Reveal>
        <div className={styles.featureGrid}>
          {features.map((item, i) => (
            <Reveal key={item.title} delay={i * 80}>
              <article className={styles.featureCard}>
                <div className={styles.featureIcon}>{item.icon}</div>
                <Heading as="h3">{item.title}</Heading>
                <p>{item.body}</p>
              </article>
            </Reveal>
          ))}
        </div>
      </div>
    </section>
  );
}

function ScoringSection() {
  return (
    <section className={styles.sectionAlt} id="scoring">
      <div className="container">
        <Reveal>
          <div className={styles.sectionHeader}>
            <span className={styles.sectionLabel}>Health scoring</span>
            <Heading as="h2">A 0–100 score built from six debt components</Heading>
            <p>
              Each component is weighted, scale-aware, and configurable.
              The score adapts its baselines to your repository size.
            </p>
          </div>
        </Reveal>

        <Reveal delay={100}>
          <div className={styles.scoreGrid}>
            {scoreComponents.map(comp => (
              <div key={comp.name} className={styles.scoreCard}>
                <div className={styles.scoreBar}>
                  <div
                    className={styles.scoreBarFill}
                    style={{
                      width: comp.weight,
                      background: `linear-gradient(90deg, ${comp.color}, ${comp.color}88)`,
                    }}
                  />
                </div>
                <div className={styles.scoreInfo}>
                  <span className={styles.scoreName} style={{color: comp.color}}>
                    {comp.name}
                  </span>
                  <span className={styles.scoreWeight}>{comp.weight}</span>
                </div>
                <p className={styles.scoreDesc}>{comp.description}</p>
              </div>
            ))}
          </div>
        </Reveal>

        <Reveal delay={200}>
          <div className={styles.scoreRanges}>
            <div className={styles.rangeItem}>
              <span className={styles.rangeScore} style={{color: '#9ece6a'}}>90–100</span>
              <span className={styles.rangeLabel}>Clean</span>
            </div>
            <div className={styles.rangeDivider} />
            <div className={styles.rangeItem}>
              <span className={styles.rangeScore} style={{color: '#7aa2f7'}}>70–89</span>
              <span className={styles.rangeLabel}>Healthy</span>
            </div>
            <div className={styles.rangeDivider} />
            <div className={styles.rangeItem}>
              <span className={styles.rangeScore} style={{color: '#e0af68'}}>40–69</span>
              <span className={styles.rangeLabel}>Warning</span>
            </div>
            <div className={styles.rangeDivider} />
            <div className={styles.rangeItem}>
              <span className={styles.rangeScore} style={{color: '#f7768e'}}>0–39</span>
              <span className={styles.rangeLabel}>Critical</span>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}

function WorkflowSection() {
  return (
    <section className={styles.section} id="workflow">
      <div className="container">
        <div className={styles.workflowLayout}>
          <div className={styles.workflowLeft}>
            <Reveal>
              <span className={styles.sectionLabel}>Navigation model</span>
              <Heading as="h2">Structure before raw detail</Heading>
              <p className={styles.workflowDesc}>
                Large repositories are easier to review when you start at the
                cluster level. MorphArch keeps the raw graph available, but only
                surfaces it when you need it.
              </p>
            </Reveal>

            <div className={styles.stepList}>
              {workflowSteps.map((step, i) => (
                <Reveal key={step.title} delay={i * 100}>
                  <div className={styles.stepItem}>
                    <div className={styles.stepNumber}>{step.number}</div>
                    <div className={styles.stepContent}>
                      <Heading as="h3">{step.title}</Heading>
                      <p>{step.body}</p>
                    </div>
                  </div>
                </Reveal>
              ))}
            </div>
          </div>

          <Reveal delay={150}>
            <aside className={styles.workflowAside}>
              <Heading as="h3">Built for repository review</Heading>
              <ul className={styles.checkList}>
                <li>
                  <span className={styles.checkIcon}>✓</span>
                  Grouped map view by default
                </li>
                <li>
                  <span className={styles.checkIcon}>✓</span>
                  Trend, hotspots, and blast in one workflow
                </li>
                <li>
                  <span className={styles.checkIcon}>✓</span>
                  Deterministic first-parent history replay
                </li>
                <li>
                  <span className={styles.checkIcon}>✓</span>
                  Configurable clustering and boundaries
                </li>
                <li>
                  <span className={styles.checkIcon}>✓</span>
                  AI assistant for architecture questions
                </li>
              </ul>
            </aside>
          </Reveal>
        </div>
      </div>
    </section>
  );
}

function AISection() {
  return (
    <section className={styles.sectionAlt} id="ai">
      <div className="container">
        <div className={styles.aiLayout}>
          <Reveal>
            <div className={styles.aiContent}>
              <span className={styles.sectionLabel}>AI Assistant</span>
              <Heading as="h2">Ask your architecture questions in plain language</Heading>
              <p className={styles.aiDesc}>
                Press <code>a</code> in the TUI to open the AI panel. It uses the full
                architecture context — health scores, dependency edges, blast radius,
                churn hotspots, cycle groups — to answer your questions.
              </p>

              <div className={styles.aiExamples}>
                <div className={styles.aiExample}>
                  <span className={styles.aiQ}>→</span>
                  "Which modules are the most fragile?"
                </div>
                <div className={styles.aiExample}>
                  <span className={styles.aiQ}>→</span>
                  "How can I break the circular dependencies?"
                </div>
                <div className={styles.aiExample}>
                  <span className={styles.aiQ}>→</span>
                  "/diff 5 — what changed in the last 5 commits?"
                </div>
              </div>

              <div className={styles.aiProviders}>
                <span className={styles.aiProviderLabel}>Compatible with</span>
                <span className={styles.aiProvider}>OpenAI</span>
                <span className={styles.aiProvider}>Ollama</span>
                <span className={styles.aiProvider}>LM Studio</span>
                <span className={styles.aiProvider}>Azure OpenAI</span>
              </div>
            </div>
          </Reveal>

          <Reveal delay={100}>
            <div className={styles.aiTerminal}>
              <div className={styles.aiTermHeader}>
                <span className={styles.dotRed} />
                <span className={styles.dotYellow} />
                <span className={styles.dotGreen} />
                <span className={styles.aiTermTitle}>AI Assistant</span>
              </div>
              <div className={styles.aiTermBody}>
                <div className={styles.aiMsg}>
                  <span className={styles.aiMsgUser}>You:</span>
                  What is the riskiest module?
                </div>
                <div className={styles.aiMsg}>
                  <span className={styles.aiMsgAI}>AI:</span>
                  <span className={styles.aiHighlight}>parser/resolver</span> has the highest blast
                  score (0.72) with 14 downstream dependents. It sits in a cycle with{' '}
                  <span className={styles.aiHighlight}>graph_builder</span> and has instability
                  I=0.89. Consider extracting the shared types into a leaf module to break
                  the cycle and reduce ripple risk.
                </div>
              </div>
            </div>
          </Reveal>
        </div>
      </div>
    </section>
  );
}

function InstallSection() {
  const [active, setActive] = useState(0);
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(installMethods[active].command);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <section className={styles.section} id="install">
      <div className="container">
        <Reveal>
          <div className={styles.sectionHeader}>
            <span className={styles.sectionLabel}>Installation</span>
            <Heading as="h2">Install in seconds, analyze in minutes</Heading>
            <p>Choose your preferred method. No build dependencies required.</p>
          </div>
        </Reveal>

        <Reveal delay={100}>
          <div className={styles.installPanel}>
            <div className={styles.installTabs}>
              {installMethods.map((m, i) => (
                <button
                  key={m.label}
                  type="button"
                  className={`${styles.installTab} ${i === active ? styles.installTabActive : ''}`}
                  onClick={() => {setActive(i); setCopied(false);}}>
                  {m.label}
                </button>
              ))}
            </div>
            <div className={styles.installBody}>
              <code className={styles.installCmd}>
                <span className={styles.installPrompt}>$</span> {installMethods[active].command}
              </code>
              <button className={styles.installCopy} onClick={handleCopy} type="button">
                {copied ? '✓' : 'Copy'}
              </button>
            </div>

            <div className={styles.installAlso}>
              Also available via{' '}
              <Link to="https://github.com/onplt/morpharch#installation">
                Docker, Scoop, AUR, and cargo-binstall
              </Link>
            </div>
          </div>
        </Reveal>

        <Reveal delay={200}>
          <div className={styles.quickRun}>
            <div className={styles.quickRunStep}>
              <span className={styles.quickRunNum}>1</span>
              <div>
                <strong>Install</strong>
                <code>cargo install morpharch</code>
              </div>
            </div>
            <div className={styles.quickRunArrow}>→</div>
            <div className={styles.quickRunStep}>
              <span className={styles.quickRunNum}>2</span>
              <div>
                <strong>Scan & open TUI</strong>
                <code>morpharch watch .</code>
              </div>
            </div>
            <div className={styles.quickRunArrow}>→</div>
            <div className={styles.quickRunStep}>
              <span className={styles.quickRunNum}>3</span>
              <div>
                <strong>Review architecture</strong>
                <code>Map → Cluster → Inspect</code>
              </div>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}

function AudienceSection() {
  const audiences = [
    {
      role: 'Architects',
      body: 'Review boundaries, coupling, and structural pressure without manually reading a full dependency graph.',
      accent: '#7aa2f7',
    },
    {
      role: 'Tech Leads',
      body: 'Track drift over time, review what changed between commits, and identify parts of the repo that are becoming harder to change.',
      accent: '#bb9af7',
    },
    {
      role: 'Developers',
      body: 'Answer practical questions — who depends on a module, what it pulls in, and what changes are likely to ripple outward.',
      accent: '#9ece6a',
    },
  ];

  return (
    <section className={styles.sectionAlt}>
      <div className="container">
        <Reveal>
          <div className={styles.sectionHeader}>
            <span className={styles.sectionLabel}>Who it's for</span>
            <Heading as="h2">Built for people who care about architecture</Heading>
          </div>
        </Reveal>
        <div className={styles.audienceGrid}>
          {audiences.map((item, i) => (
            <Reveal key={item.role} delay={i * 80}>
              <article className={styles.audienceCard}>
                <div className={styles.audienceAccent} style={{background: item.accent}} />
                <Heading as="h3">{item.role}</Heading>
                <p>{item.body}</p>
              </article>
            </Reveal>
          ))}
        </div>
      </div>
    </section>
  );
}

function OpenSourceSection() {
  return (
    <section className={styles.section}>
      <div className="container">
        <Reveal>
          <div className={styles.ossPanel}>
            <div className={styles.ossContent}>
              <span className={styles.sectionLabel}>Open source</span>
              <Heading as="h2">Built in the open</Heading>
              <p>
                MorphArch is Apache-2.0 / MIT licensed. Contributions around language
                support, clustering, scan correctness, TUI polish, and documentation
                are welcome.
              </p>
              <div className={styles.ossActions}>
                <Link className={styles.btnPrimary} to="https://github.com/onplt/morpharch">
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z" />
                  </svg>
                  View on GitHub
                </Link>
                <Link className={styles.btnGhost} to="/docs/intro">
                  Read the docs
                </Link>
              </div>
            </div>
            <div className={styles.ossBadges}>
              <div className={styles.ossBadge}>
                <span className={styles.ossBadgeValue}>Apache-2.0 / MIT</span>
                <span className={styles.ossBadgeLabel}>License</span>
              </div>
              <div className={styles.ossBadge}>
                <span className={styles.ossBadgeValue}>5 languages</span>
                <span className={styles.ossBadgeLabel}>Supported</span>
              </div>
              <div className={styles.ossBadge}>
                <span className={styles.ossBadgeValue}>Pure Rust</span>
                <span className={styles.ossBadgeLabel}>No git CLI</span>
              </div>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}

function FinalCTA() {
  return (
    <section className={styles.finalCta}>
      <div className="container">
        <Reveal>
          <div className={styles.ctaInner}>
            <div className={styles.ctaGlow} />
            <Heading as="h2">Start with one scan</Heading>
            <p>
              Install MorphArch, point it at a repository, and see architecture
              health in under a minute.
            </p>
            <div className={styles.ctaActions}>
              <Link className={styles.btnPrimary} to="/docs/quick-start">
                Quick start guide
              </Link>
              <Link className={styles.btnGhost} to="/docs/guides/configuration">
                Configuration reference
              </Link>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}

/* ─── Page ─────────────────────────────────────────────────────────────────── */

export default function Home(): ReactNode {
  const {siteConfig} = useDocusaurusContext();

  return (
    <Layout
      title={`${siteConfig.title} — Repository architecture health from the terminal`}
      description="MorphArch scans Git history, builds dependency graphs, computes health scores, and helps you inspect structure, drift, and hotspots from a terminal UI.">
      <main className={styles.page}>
        <Hero />
        <CompatibilityBar />
        <FeatureSection />
        <ScoringSection />
        <WorkflowSection />
        <AISection />
        <InstallSection />
        <AudienceSection />
        <OpenSourceSection />
        <FinalCTA />
      </main>
    </Layout>
  );
}
