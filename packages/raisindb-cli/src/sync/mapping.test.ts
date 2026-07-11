import { describe, it, expect } from 'vitest';
import {
  mapChangeToNode,
  parseTranslationLocale,
} from './mapping.js';

describe('mapChangeToNode', () => {
  describe('.node.yaml files (directory nodes)', () => {
    it('maps a function .node.yaml to the function node', () => {
      const m = mapChangeToNode('functions/lib/shiftboard/list-shifts/.node.yaml');
      expect(m.kind).toBe('node-yaml');
      expect(m.workspace).toBe('functions');
      expect(m.nodePath).toBe('lib/shiftboard/list-shifts');
    });

    it('maps a workspace-root .node.yaml folder node', () => {
      const m = mapChangeToNode('staffing/shifts/.node.yaml');
      expect(m.kind).toBe('node-yaml');
      expect(m.workspace).toBe('staffing');
      expect(m.nodePath).toBe('shifts');
    });

    it('decodes namespaced workspaces', () => {
      const m = mapChangeToNode('_raisin__access_control/inbox/.node.yaml');
      expect(m.kind).toBe('node-yaml');
      expect(m.workspace).toBe('raisin:access_control');
      expect(m.nodePath).toBe('inbox');
    });
  });

  describe('named node YAML files', () => {
    it('strips the .yaml extension from the node path', () => {
      const m = mapChangeToNode('staffing/shifts/fri-evening.yaml');
      expect(m.kind).toBe('node-file');
      expect(m.workspace).toBe('staffing');
      expect(m.nodePath).toBe('shifts/fri-evening');
    });

    it('strips the .yml extension too', () => {
      const m = mapChangeToNode('staffing/staff/anna.yml');
      expect(m.kind).toBe('node-file');
      expect(m.nodePath).toBe('staff/anna');
    });

    it('prefers an explicit name over the filename stem', () => {
      const m = mapChangeToNode('staffing/shifts/fri-evening.yaml', 'friday-evening');
      expect(m.nodePath).toBe('shifts/friday-evening');
    });
  });

  describe('code files', () => {
    it('maps index.js to a code asset under the function node', () => {
      const m = mapChangeToNode('functions/lib/shiftboard/list-shifts/index.js');
      expect(m.kind).toBe('code');
      expect(m.workspace).toBe('functions');
      expect(m.nodePath).toBe('lib/shiftboard/list-shifts/index.js');
    });

    it('maps .py and .star files as code', () => {
      expect(mapChangeToNode('functions/lib/tool/main.py').kind).toBe('code');
      expect(mapChangeToNode('functions/lib/tool/policy.star').kind).toBe('code');
    });
  });

  describe('translations', () => {
    it('maps .node.de.yaml to a translation of the directory node', () => {
      const m = mapChangeToNode('launchpad/home/.node.de.yaml');
      expect(m.kind).toBe('translation');
      expect(m.locale).toBe('de');
      expect(m.workspace).toBe('launchpad');
      expect(m.nodePath).toBe('home');
    });

    it('maps about.fr.yaml to a translation of the named node', () => {
      const m = mapChangeToNode('launchpad/pages/about.fr.yaml');
      expect(m.kind).toBe('translation');
      expect(m.locale).toBe('fr');
      expect(m.nodePath).toBe('pages/about');
    });
  });

  describe('structural changes (require re-deploy)', () => {
    it('flags manifest.yaml', () => {
      const m = mapChangeToNode('manifest.yaml');
      expect(m.kind).toBe('structural');
      expect(m.reason).toContain('deploy --install');
    });

    it('flags workspaces/', () => {
      const m = mapChangeToNode('workspaces/staffing.yaml');
      expect(m.kind).toBe('structural');
    });
  });

  describe('schema changes (live-synced to the management API)', () => {
    it('classifies nodetypes/ as schema', () => {
      const m = mapChangeToNode('nodetypes/shift.yaml');
      expect(m.kind).toBe('schema');
      expect(m.schemaKind).toBe('nodetype');
    });

    it('classifies mixins/ and archetypes/ as schema', () => {
      const mixin = mapChangeToNode('mixins/audited.yaml');
      expect(mixin.kind).toBe('schema');
      expect(mixin.schemaKind).toBe('mixin');

      const archetype = mapChangeToNode('archetypes/page.yaml');
      expect(archetype.kind).toBe('schema');
      expect(archetype.schemaKind).toBe('archetype');
    });
  });

  describe('skipped files', () => {
    it('skips asset metadata files (.node.index.js.yaml)', () => {
      const m = mapChangeToNode('functions/lib/tool/.node.index.js.yaml');
      expect(m.kind).toBe('skip');
    });

    it('skips hidden files', () => {
      expect(mapChangeToNode('staffing/.DS_Store').kind).toBe('skip');
    });

    it('skips root-level non-manifest files', () => {
      expect(mapChangeToNode('README.md').kind).toBe('skip');
    });

    it('skips empty paths', () => {
      expect(mapChangeToNode('').kind).toBe('skip');
    });
  });

  describe('binary assets', () => {
    it('maps other files to asset uploads with full filename', () => {
      const m = mapChangeToNode('launchpad/images/logo.png');
      expect(m.kind).toBe('asset');
      expect(m.workspace).toBe('launchpad');
      expect(m.nodePath).toBe('images/logo.png');
    });

    it('treats markdown content files as assets', () => {
      const m = mapChangeToNode('docs/guides/intro.md');
      expect(m.kind).toBe('asset');
      expect(m.nodePath).toBe('guides/intro.md');
    });
  });

  it('normalizes windows-style separators', () => {
    const m = mapChangeToNode('functions\\lib\\tool\\index.js');
    expect(m.kind).toBe('code');
    expect(m.nodePath).toBe('lib/tool/index.js');
  });
});

describe('parseTranslationLocale', () => {
  it('parses .node.<locale>.yaml', () => {
    expect(parseTranslationLocale('.node.de.yaml')).toBe('de');
    expect(parseTranslationLocale('.node.fr.yaml')).toBe('fr');
    expect(parseTranslationLocale('.node.pt-BR.yaml')).toBe('pt-BR');
  });

  it('parses <name>.<locale>.yaml', () => {
    expect(parseTranslationLocale('about.de.yaml')).toBe('de');
  });

  it('rejects non-translation files', () => {
    expect(parseTranslationLocale('.node.yaml')).toBeNull();
    expect(parseTranslationLocale('.node.index.js.yaml')).toBeNull();
    expect(parseTranslationLocale('about.yaml')).toBeNull();
    expect(parseTranslationLocale('index.js')).toBeNull();
  });
});
