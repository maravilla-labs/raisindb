import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

/**
 * Sidebars organized following the Diátaxis framework:
 * - Tutorials (separate nav) → Learning-oriented
 * - Docs → Explanation (Understand) + How-to Guides
 * - Reference (part of Docs) → Information-oriented
 *
 * @see https://diataxis.fr
 */
const sidebars: SidebarsConfig = {
  // ==========================================================================
  // TUTORIALS - Learning-oriented
  // "Follow along with me" - Hand-holding lessons for beginners
  // ==========================================================================
  tutorialsSidebar: [
    'tutorials/overview',
    'tutorials/quickstart/index',
    {
      type: 'category',
      label: 'News Feed',
      link: { type: 'doc', id: 'tutorials/news-feed/index' },
      items: [
        'tutorials/news-feed/sveltekit',
        'tutorials/news-feed/spring-boot',
        'tutorials/news-feed/laravel',
      ],
    },
    'tutorials/iot-dashboard/index',
    'tutorials/shift-planner/index',
  ],

  // ==========================================================================
  // DOCS - Explanation + How-to + Reference combined
  // ==========================================================================
  tutorialSidebar: [
    // ------------------------------------------------------------------------
    // UNDERSTAND (Explanation) - "Why does this work this way?"
    // Background, context, design decisions
    // ------------------------------------------------------------------------
    {
      type: 'category',
      label: 'Understand',
      items: [
        'why/overview',
        'why/concepts',
        {
          type: 'category',
          label: 'Architecture',
          items: [
            'why/architecture',
            'why/architecture/document-storage',
            'why/architecture/comparisons',
          ],
        },
      ],
    },

    // ------------------------------------------------------------------------
    // HOW-TO GUIDES - Task-oriented
    // "Here's how to do X" - Practical steps for specific problems
    // ------------------------------------------------------------------------
    {
      type: 'category',
      label: 'How-to Guides',
      items: [
        'getting-started/installation',
        'getting-started/cluster',
        {
          type: 'category',
          label: 'Model Your Data',
          items: [
            'model/overview',
            'model/nodetypes/overview',
          ],
        },
        {
          type: 'category',
          label: 'Query Data',
          items: [
            'access/sql/overview',
            'access/sql/examples',
          ],
        },
        {
          type: 'category',
          label: 'Secure Your Data',
          items: [
            'access/security/overview',
            'access/security/examples',
          ],
        },
        'operate/overview',
      ],
    },

    // ------------------------------------------------------------------------
    // REFERENCE - Information-oriented
    // Dry, factual, complete - Not meant to teach, just to look things up
    // ------------------------------------------------------------------------
    {
      type: 'category',
      label: 'Reference',
      items: [
        {
          type: 'category',
          label: 'NodeTypes',
          items: [
            'model/nodetypes/property-types',
          ],
        },
        {
          type: 'category',
          label: 'SQL',
          items: [
            'access/sql/raisinsql',
            'access/sql/branches',
            'access/sql/restore',
            'access/sql/graph_table',
            'access/sql/cypher',
            'access/sql/fulltext',
          ],
        },
        {
          type: 'category',
          label: 'REST API',
          items: [
            'access/rest/overview',
            'access/rest/shapes',
            'access/rest/translations',
            'access/rest/examples',
          ],
        },
        {
          type: 'category',
          label: 'WebSocket',
          items: [
            'access/websocket/overview',
          ],
        },
        {
          type: 'category',
          label: 'Security',
          items: [
            'access/security/conditions',
          ],
        },
        'reference/errors',
        'reference/faq',
      ],
    },
  ],
};

export default sidebars;
