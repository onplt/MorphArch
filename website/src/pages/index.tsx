import type {ReactNode} from 'react';
import React, {useEffect} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import HomepageFeatures from '@site/src/components/HomepageFeatures';
import Heading from '@theme/Heading';

import styles from './index.module.css';

// Intersection Observer Hook for Scroll Animations
function useScrollReveal() {
  useEffect(() => {
    const observer = new IntersectionObserver((entries) => {
      entries.forEach(entry => {
        if (entry.isIntersecting) {
          entry.target.classList.add('visible');
        }
      });
    }, { threshold: 0.1 });

    document.querySelectorAll('.reveal').forEach(el => observer.observe(el));
    return () => observer.disconnect();
  }, []);
}

import useBaseUrl from '@docusaurus/useBaseUrl';

function TerminalMockup() {
  return (
    <div className="terminal-window">
      <div className="terminal-header">
        <div className="dot red"></div>
        <div className="dot yellow"></div>
        <div className="dot green"></div>
        <div style={{ marginLeft: '10px', fontSize: '12px', color: '#565f89' }}>morpharch demo</div>
      </div>
      <img 
        src={useBaseUrl('/img/demo.gif')} 
        alt="MorphArch TUI Demo" 
        style={{ width: '100%', display: 'block' }} 
      />
    </div>
  );
}

function Compatibility() {
  return (
    <section className="compatibility-section reveal">
      <div className="container">
        <div className="comp-grid">
          <span style={{ color: '#565f89', fontWeight: 'bold' }}>WORKS WITH:</span>
          <div className="comp-item">Nx</div>
          <div className="comp-item">Turborepo</div>
          <div className="comp-item">pnpm</div>
          <div className="comp-item">Cargo</div>
          <div className="comp-item">Lerna</div>
        </div>
      </div>
    </section>
  );
}

function VisualComparison() {
  return (
    <section style={{ padding: '8rem 0', background: '#16161e' }}>
      <div className="container">
        <div className="section-title reveal">
          <h2>Transform Your Architecture</h2>
          <p style={{ color: '#9aa5ce' }}>Turn chaotic dependency graphs into structured, maintainable systems.</p>
        </div>
        <div className="comparison-container reveal">
          <div className="comparison-card">
            <img src="/img/arch-before.svg" alt="Spaghetti Architecture" className="comparison-image" />
            <p style={{ marginTop: '1rem', color: '#f7768e', fontWeight: 'bold' }}>Chaotic & Cyclic</p>
          </div>
          <div className="comparison-arrow">→</div>
          <div className="comparison-card">
            <img src="/img/arch-after.svg" alt="Clean Architecture" className="comparison-image" />
            <p style={{ marginTop: '1rem', color: '#9ece6a', fontWeight: 'bold' }}>Organized & Layered</p>
          </div>
        </div>
      </div>
    </section>
  );
}

function BuiltForScale() {
  return (
    <section style={{ padding: '8rem 0', background: '#1a1b26' }}>
      <div className="container">
        <div className="section-title reveal">
          <h2>Architected for Performance</h2>
          <p style={{ color: '#9aa5ce', maxWidth: '700px', margin: '0 auto' }}>
            MorphArch is written in 100% pure Rust. It handles the largest monorepos without breaking a sweat.
          </p>
        </div>
        <div className="perf-grid reveal">
          <div className="perf-card">
            <span className="perf-value">&lt; 10s</span>
            <span className="perf-label">Scan 1000 Commits</span>
          </div>
          <div className="perf-card">
            <span className="perf-value">50k</span>
            <span className="perf-label">File LRU Cache</span>
          </div>
          <div className="perf-card">
            <span className="perf-value">∞</span>
            <span className="perf-label">Parallel AST Parsing</span>
          </div>
        </div>
      </div>
    </section>
  );
}

function Roadmap() {
  const items = [
    { status: 'In Progress', title: 'Web-Based Dashboard', desc: 'A rich browser interface for historical drift analysis.' },
    { status: 'Planned', title: 'Slack & Discord Alerts', desc: 'Get notified when architecture health drops in a PR.' },
    { status: 'Planned', title: 'Language Server (LSP)', desc: 'Real-time boundary violation warnings in your IDE.' },
  ];

  return (
    <section style={{ padding: '8rem 0', background: '#16161e' }}>
      <div className="container">
        <div className="section-title reveal">
          <h2>Future Vision</h2>
          <p style={{ color: '#9aa5ce' }}>Where MorphArch is headed next.</p>
        </div>
        <div className="roadmap-grid">
          {items.map((item, idx) => (
            <div key={idx} className="roadmap-item reveal" style={{ transitionDelay: `${idx * 0.1}s` }}>
              <span className={clsx('roadmap-status', item.status === 'Planned' ? 'status-planned' : 'status-progress')}>
                {item.status}
              </span>
              <div>
                <Heading as="h4" style={{ margin: 0, color: '#c0caf5' }}>{item.title}</Heading>
                <p style={{ margin: 0, color: '#565f89', fontSize: '0.9rem' }}>{item.desc}</p>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function Personas() {
  const personas = [
    {
      icon: '🏗️',
      title: 'Architects',
      desc: 'Define and enforce boundary rules. Ensure the codebase follows the intended dependency flow.'
    },
    {
      icon: '🚀',
      title: 'Team Leads',
      desc: 'Monitor architectural health trends. Prevent spaghetti code before it slows down the team.'
    },
    {
      icon: '🛠️',
      title: 'Developers',
      desc: 'Understand complex package relationships instantly without leaving the terminal.'
    }
  ];

  return (
    <section className="persona-section">
      <div className="container">
        <div className="section-title reveal">
          <h2>Who is MorphArch for?</h2>
        </div>
        <div className="row">
          {personas.map((p, idx) => (
            <div key={idx} className="col col--4 reveal" style={{ transitionDelay: `${idx * 0.1}s` }}>
              <div className="persona-card">
                <span className="persona-icon">{p.icon}</span>
                <Heading as="h3" style={{ color: '#7aa2f7' }}>{p.title}</Heading>
                <p style={{ color: '#a9b1d6', fontSize: '0.95rem' }}>{p.desc}</p>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function Contribute() {
  return (
    <section className="contribute-section reveal">
      <div className="container">
        <h2>Open Source to the Core</h2>
        <p style={{ color: '#9aa5ce', maxWidth: '600px', margin: '0 auto' }}>
          MorphArch is built by and for the community. Whether it's adding a new language grammar or fixing a bug, your help is welcome.
        </p>
        <a href="https://github.com/onplt/morpharch" className="github-card">
          <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
            <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.744.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.44-1.304.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/>
          </svg>
          <span>Star us on GitHub</span>
        </a>
      </div>
    </section>
  );
}

function HowItWorks() {
  const steps = [
    {
      title: 'Scan',
      desc: 'MorphArch walks your Git history and detects workspace boundaries (Nx, Turbo, Cargo) automatically.',
    },
    {
      title: 'Parse',
      desc: 'Using Tree-sitter, it performs deep AST analysis to extract real dependency edges from your source code.',
    },
    {
      title: 'Visualize',
      desc: 'See your architecture evolve in real-time through an interactive, physics-based terminal UI.',
    },
  ];

  return (
    <section style={{ padding: '6rem 0', background: '#1a1b26' }}>
      <div className="container">
        <div className="section-title reveal">
          <h2>How it Works</h2>
          <p style={{ color: '#9aa5ce' }}>Analyze architectural drift in three simple steps.</p>
        </div>
        <div className="row">
          {steps.map((step, idx) => (
            <div key={idx} className="col col--4 reveal" style={{ marginBottom: '2rem', transitionDelay: `${idx * 0.2}s` }}>
              <div className="step-card">
                <div className="step-number">{idx + 1}</div>
                <Heading as="h3" style={{ color: '#bb9af7', marginTop: '1rem' }}>{step.title}</Heading>
                <p style={{ color: '#a9b1d6', fontSize: '0.95rem' }}>{step.desc}</p>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function FAQ() {
  const faqs = [
    {
      q: "Is it fast enough for large monorepos?",
      a: "Yes. MorphArch uses subtree-cached walks and a 50K-entry LRU blob cache. Subsequent scans are up to 20x faster."
    },
    {
      q: "Does it support my programming language?",
      a: "Out of the box, we support Rust, TypeScript, JavaScript, Python, and Go via high-precision AST parsing."
    },
    {
      q: "How secure is MorphArch?",
      a: "Completely. It runs entirely on your machine. No code is ever uploaded to any server. Data is stored in a local SQLite file."
    }
  ];

  return (
    <section className="faq-section">
      <div className="container">
        <div className="section-title reveal">
          <h2>Frequently Asked Questions</h2>
        </div>
        <div className="row">
          {faqs.map((faq, idx) => (
            <div key={idx} className="col col--4 reveal" style={{ transitionDelay: `${idx * 0.1}s` }}>
              <div className="faq-item">
                <h3>{faq.q}</h3>
                <p>{faq.a}</p>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function BottomCTA() {
  return (
    <section className="bottom-cta reveal">
      <div className="container">
        <h2>Ready to secure your architecture?</h2>
        <p style={{ color: '#9aa5ce', marginBottom: '2rem', fontSize: '1.2rem' }}>
          Stop the drift today. Install MorphArch and take control of your codebase.
        </p>
        <div className="install-command-wrapper" style={{ justifyContent: 'center' }}>
          <div className="install-command" style={{ margin: 0 }}>
            <code>$ cargo install morpharch</code>
          </div>
        </div>
        <div style={{ marginTop: '2rem' }}>
          <Link
            className="button button--primary button--lg"
            to="/docs/intro"
            style={{ borderRadius: '8px', padding: '15px 40px', fontWeight: 'bold' }}>
            Get Started for Free
          </Link>
        </div>
      </div>
    </section>
  );
}

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  
  const handleCopy = () => {
    navigator.clipboard.writeText('cargo install morpharch');
    const btn = document.querySelector('.copy-button');
    if (btn) {
      const originalText = btn.textContent;
      btn.textContent = 'Copied!';
      setTimeout(() => btn.textContent = originalText, 2000);
    }
  };

  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <div className="hero-content">
          <div className="hero-text reveal">
            <div className="badges">
              <a href="https://crates.io/crates/morpharch" target="_blank" rel="noopener noreferrer">
                <img src="https://img.shields.io/crates/v/morpharch?color=7aa2f7&style=flat-square" alt="Crates.io" />
              </a>
              <a href="https://github.com/onplt/morpharch" target="_blank" rel="noopener noreferrer" style={{ marginLeft: '10px' }}>
                <img src="https://img.shields.io/github/stars/onplt/morpharch?color=bb9af7&style=flat-square" alt="GitHub Stars" />
              </a>
            </div>
            <Heading as="h1" className="hero__title">
              {siteConfig.title}
            </Heading>
            <p className="hero__subtitle">
              Understand your monorepo's architectural evolution with AST-powered dependency analysis and an animated TUI.
            </p>
            
            <div className="install-command-wrapper">
              <div className="install-command" style={{ margin: 0 }}>
                <code>$ cargo install morpharch</code>
              </div>
              <button className="copy-button" onClick={handleCopy}>Copy</button>
            </div>

            <div className={styles.buttons} style={{ marginBottom: '3rem' }}>
              <Link
                className="button button--primary button--lg"
                to="/docs/intro"
                style={{ borderRadius: '8px', padding: '12px 32px', fontWeight: 'bold', boxShadow: '0 4px 15px rgba(122, 162, 247, 0.3)' }}>
                Start Analyzing 🚀
              </Link>
            </div>

            <div className="ecosystem-section">
              <div className="ecosystem-logos">
                <span>SUPPORTED:</span>
                <span className="ecosystem-badge" style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <img src="https://simpleicons.org/icons/rust.svg" style={{ width: '16px', filter: 'invert(1) brightness(2)' }} /> Rust
                </span>
                <span className="ecosystem-badge" style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <img src="https://simpleicons.org/icons/typescript.svg" style={{ width: '16px' }} /> TypeScript
                </span>
                <span className="ecosystem-badge" style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <img src="https://simpleicons.org/icons/python.svg" style={{ width: '16px' }} /> Python
                </span>
                <span className="ecosystem-badge" style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <img src="https://simpleicons.org/icons/go.svg" style={{ width: '16px' }} /> Go
                </span>
              </div>
            </div>
          </div>
          
          <div className="reveal" style={{ transitionDelay: '0.3s' }}>
            <TerminalMockup />
          </div>
        </div>
      </div>
    </header>
  );
}

export default function Home(): ReactNode {
  const {siteConfig} = useDocusaurusContext();
  useScrollReveal();

  return (
    <Layout
      title={`${siteConfig.title} | Visualizing Architecture Evolution`}
      description="Monorepo architecture drift visualizer with animated TUI and health scoring.">
      <HomepageHeader />
      <main>
        <Compatibility />
        <div className="reveal">
          <HomepageFeatures />
        </div>
        <VisualComparison />
        <BuiltForScale />
        <HowItWorks />
        <Roadmap />
        <Personas />
        <Contribute />
        <FAQ />
        <BottomCTA />
      </main>
    </Layout>
  );
}
