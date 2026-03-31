import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

// This runs in Node.js - Don't use client-side code here (browser APIs, JSX...)

const config: Config = {
  title: 'RaisinDB',
  tagline: 'Document Database with Git-like Versioning',
  favicon: 'img/favicon.ico',

  // Future flags, see https://docusaurus.io/docs/api/docusaurus-config#future
  future: {
    v4: true, // Improve compatibility with the upcoming Docusaurus v4
  },

  // Set the production url of your site here
  url: 'https://raisindb.dev',
  // Set the /<baseUrl>/ pathname under which your site is served
  // For GitHub pages deployment, it is often '/<projectName>/'
  baseUrl: '/',

  // Load Inter from Google Fonts
  stylesheets: [
    {
      href: 'https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700;800&display=swap',
      type: 'text/css',
    },
  ],

  // GitHub pages deployment config.
  // If you aren't using GitHub pages, you don't need these.
  organizationName: 'solutas', // Usually your GitHub org/user name.
  projectName: 'raisindb', // Usually your repo name.

  onBrokenLinks: 'throw',

  // Even if you don't use internationalization, you can use this field to set
  // useful metadata like html lang. For example, if your site is Chinese, you
  // may want to replace "en" with "zh-Hans".
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
          // Please change this to your repo.
          // Remove this to remove the "edit this page" links.
          editUrl:
            'https://github.com/maravilla-labs/raisindb/tree/main/docs/website/',
        },
        blog: false, // Disable blog for now
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    // Replace with your project's social card
    image: 'img/docusaurus-social-card.jpg',
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'RaisinDB',
      logo: {
        alt: 'RaisinDB Logo',
        src: 'img/raisin-logo.png',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'tutorialsSidebar',
          position: 'left',
          label: 'Tutorials',
        },
        {
          type: 'docSidebar',
          sidebarId: 'tutorialSidebar',
          position: 'left',
          label: 'Docs',
        },
        {
          type: 'doc',
          docId: 'access/rest/overview',
          position: 'left',
          label: 'API',
        },
        {
          href: 'https://github.com/maravilla-labs/raisindb',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Learn',
          items: [
            {
              label: 'Tutorials',
              to: '/docs/tutorials/overview',
            },
            {
              label: 'Getting Started',
              to: '/docs/getting-started/quickstart',
            },
            {
              label: 'Concepts',
              to: '/docs/why/concepts',
            },
          ],
        },
        {
          title: 'Documentation',
          items: [
            {
              label: 'API Reference',
              to: '/docs/access/rest/overview',
            },
          ],
        },
        {
          title: 'Community',
          items: [
            {
              label: 'GitHub',
              href: 'https://github.com/maravilla-labs/raisindb',
            },
            {
              label: 'Issues',
              href: 'https://github.com/maravilla-labs/raisindb/issues',
            },
          ],
        },
        {
          title: 'Legal',
          items: [
            {
              label: 'License',
              to: '/docs/reference/faq#license',
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} SOLUTAS GmbH. Licensed under Business Source License 1.1.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
