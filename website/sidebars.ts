import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  tutorialSidebar: [
    {
      type: 'category',
      label: 'Getting Started',
      collapsed: false,
      items: [
        'intro',
        'installation',
        'quick-start',
      ],
    },
    {
      type: 'category',
      label: 'Core Concepts',
      items: [
        'concepts/how-it-works',
        'concepts/scoring',
        'concepts/ast-parsing',
      ],
    },
    {
      type: 'category',
      label: 'Guides',
      items: [
        'guides/configuration',
        'guides/ci-cd-integration',
        'guides/security',
      ],
    },
    'cli-reference',
  ],
};

export default sidebars;
