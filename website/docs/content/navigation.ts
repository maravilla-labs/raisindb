export interface DocsNavSection {
  title: string;
  links: { href: string; label: string; description: string }[];
}

export const docsNav: DocsNavSection[] = [
  {
    title: 'Foundations',
    links: [
      {
        href: '/getting-started',
        label: 'Getting started',
        description: 'Connect, authenticate, and choose the right transport mode.'
      },
    ],
  },
  {
    title: 'Model & Governance',
    links: [
      {
        href: '/multi-model',
        label: 'Git-like data model',
        description: 'NodeTypes, Archetypes, ElementTypes, and branching workflows.'
      },
    ],
  },
  {
    title: 'APIs',
    links: [
      {
        href: '/client-sdk',
        label: 'Client SDK',
        description: 'Browser, server, and hybrid SDK usage patterns.'
      },
      {
        href: '/http-rest',
        label: 'HTTP REST',
        description: 'Stable REST surface for automation and servers.'
      },
      {
        href: '/websocket-streaming',
        label: 'Realtime',
        description: 'Stateful WebSocket sessions, events, and subscriptions.'
      },
    ],
  },
  {
    title: 'Query Engine',
    links: [
      {
        href: '/sql',
        label: 'SQL & search',
        description: 'Hierarchical SQL, JSON operators, vectors, and graph joins.'
      },
    ],
  },
];
