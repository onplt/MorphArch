import type {ReactNode} from 'react';
import clsx from 'clsx';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

type FeatureItem = {
  title: string;
  icon: string;
  color: string;
  description: ReactNode;
};

const FeatureList: FeatureItem[] = [
  {
    title: 'AST-Native Analysis',
    icon: 'M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z', // SVG path for search
    color: '#7aa2f7',
    description: (
      <>
        Deeply understand your monorepo. MorphArch uses Tree-sitter to extract 
        real dependency edges for Rust, TypeScript, Python, and Go with surgical precision.
      </>
    ),
  },
  {
    title: 'Animated TUI',
    icon: 'M13 10V3L4 14h7v7l9-11h-7z', // SVG path for bolt
    color: '#bb9af7',
    description: (
      <>
        Architecture is alive. Visualize your package dependencies in a high-performance, 
        physics-based terminal UI that responds to every change in real-time.
      </>
    ),
  },
  {
    title: 'Health Scoring',
    icon: 'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2h2a2 2 0 002-2zM17 19v-2a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2zm0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z', // SVG path for chart
    color: '#9ece6a',
    description: (
      <>
        Quantify your structural debt. A 6-component, scale-aware algorithm detects 
        cycles, God-modules, and tight coupling to keep your architecture maintainable.
      </>
    ),
  },
];

function Feature({title, icon, color, description}: FeatureItem) {
  return (
    <div className={clsx('col col--4')}>
      <div className="text--center">
        <div className="featureIcon" style={{ display: 'inline-block' }}>
          <svg 
            width="40" 
            height="40" 
            viewBox="0 0 24 24" 
            fill="none" 
            stroke={color} 
            strokeWidth="2" 
            strokeLinecap="round" 
            strokeLinejoin="round"
          >
            <path d={icon} />
          </svg>
        </div>
      </div>
      <div className="text--center padding-horiz--md">
        <Heading as="h3" style={{ color: color, marginTop: '1rem', fontWeight: '800' }}>{title}</Heading>
        <p style={{ color: '#9aa5ce', fontSize: '0.95rem', lineHeight: '1.6' }}>{description}</p>
      </div>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features} style={{ padding: '8rem 0', background: '#16161e' }}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
