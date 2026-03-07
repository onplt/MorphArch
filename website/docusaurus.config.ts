import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'MorphArch',
  tagline: 'Visualizing Software Architecture Evolution',
  favicon: 'img/favicon.svg',

  url: 'https://morpharch.dev',
  baseUrl: '/',

  // SEO and Metadata
  customFields: {
    description: 'High-performance monorepo architecture drift visualizer with animated TUI and health scoring.',
  },

  onBrokenLinks: 'throw',
  onBrokenMarkdownLinks: 'warn',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          editUrl: 'https://github.com/onplt/morpharch/tree/main/website/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  plugins: [
    [
      require.resolve("@easyops-cn/docusaurus-search-local"),
      {
        hashed: true,
        language: ["en"],
        highlightSearchTermsOnTargetPage: true,
        explicitSearchResultPath: true,
      },
    ],
  ],

  themeConfig: {
    image: 'img/social-card.svg',
    metadata: [
      {name: 'keywords', content: 'monorepo, architecture, rust, visualizer, tech-debt, git'},
      {name: 'twitter:card', content: 'summary_large_image'},
    ],
    navbar: {
      title: 'MorphArch',
      logo: {
        alt: 'MorphArch Logo',
        src: 'img/logo.svg',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'tutorialSidebar',
          position: 'left',
          label: 'Docs',
        },
        {
          href: 'https://github.com/onplt/morpharch',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            {
              label: 'Getting Started',
              to: '/docs/intro',
            },
            {
              label: 'Scoring Engine',
              to: '/docs/concepts/scoring',
            },
          ],
        },
        {
          title: 'Community',
          items: [
            {
              label: 'GitHub',
              href: 'https://github.com/onplt/morpharch',
            },
          ],
        },
        {
          title: 'More',
          items: [
            {
              label: 'Crates.io',
              href: 'https://crates.io/crates/morpharch',
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} MorphArch.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['rust', 'bash', 'toml'],
    },
    colorMode: {
      defaultMode: 'dark',
      respectPrefersColorScheme: true,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
