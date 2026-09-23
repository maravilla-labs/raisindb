import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { validateTranslationFiles } from './translation-validator.js';
import {
  compareVersions,
  describeTypeCatalogs,
  loadTypeCatalogs,
  parseTypeCatalog,
  type LoadedTypeCatalogs,
} from './type-catalog.js';
import { ErrorCodes } from './types.js';
// @ts-expect-error — plain .mjs build script, no type declarations
import { buildCatalog } from '../../scripts/build-builtin-catalog.mjs';

const BASE = 'content/site/page/.node.yaml';
const DE = 'content/site/page/.node.de.yaml';

/** A Studio catalog fixture: the part of standard:* the tests lean on. */
const STUDIO_CATALOG = {
  format: 'raisin-type-catalog',
  version: 1,
  source: 'studio',
  package_version: '0.3.10',
  generated_at: '2026-09-23T00:00:00.000Z',
  archetypes: [
    {
      name: 'standard:ContentPage',
      base_node_type: 'studio:Page',
      fields: [
        { $type: 'TextField', name: 'title', translatable: true },
        { $type: 'TextField', name: 'description', translatable: true },
        { $type: 'TextField', name: 'meta_title', translatable: true },
        { $type: 'TextField', name: 'canonical_url' },
        { $type: 'BooleanField', name: 'no_index' },
        { $type: 'SectionField', name: 'content' },
      ],
    },
    {
      name: 'standard:ArticlePage',
      base_node_type: 'studio:Page',
      extends: 'standard:ContentPage',
      fields: [{ $type: 'TextField', name: 'byline', translatable: true }],
    },
  ],
  elementTypes: [
    {
      name: 'standard:Switcher',
      extends: 'studio:Component',
      fields: [{ $type: 'SectionField', name: 'variants' }],
    },
    {
      name: 'standard:Variant',
      fields: [
        { $type: 'TextField', name: 'label', translatable: true },
        { $type: 'TextField', name: 'rule' },
        { $type: 'SectionField', name: 'content' },
      ],
    },
    {
      name: 'standard:Card',
      fields: [{ $type: 'ElementField', name: 'address' }],
    },
    {
      name: 'standard:Hero',
      fields: [
        { $type: 'TextField', name: 'heading', translatable: true },
        { $type: 'OptionsField', name: 'variant' },
        {
          $type: 'CompositeField',
          name: 'actions',
          fields: [
            { $type: 'TextField', name: 'label', translatable: true },
            { $type: 'TextField', name: 'href' },
          ],
        },
      ],
    },
  ],
  nodeTypes: [
    { name: 'studio:Page', properties: [{ name: 'title', is_translatable: true, type: 'String' }] },
  ],
};

let tmp: string;
let catalogDir: string;
const NO_BUILTIN = [] as string[];

function writeCatalog(file: string, content: unknown) {
  fs.writeFileSync(
    path.join(catalogDir, file),
    typeof content === 'string' ? content : JSON.stringify(content),
  );
}

function load(): LoadedTypeCatalogs {
  return loadTypeCatalogs({ catalogDir, builtinPaths: NO_BUILTIN });
}

function notTranslatable(files: Record<string, string>, catalogs: LoadedTypeCatalogs): string[] {
  const results = validateTranslationFiles({ [DE]: files[DE] }, files, catalogs.schemas);
  return results[DE].errors
    .filter(e => e.error_code === ErrorCodes.TRANSLATION_FIELD_NOT_TRANSLATABLE)
    .map(e => e.field_path);
}

const MARKETING_EXTENDS_CONTENT_PAGE = [
  'name: pkg:MarketingPage',
  'base_node_type: studio:Page',
  'extends: standard:ContentPage',
  'fields:',
  '  - { $type: TextField, name: tagline, translatable: true }',
].join('\n');

beforeEach(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-type-catalog-test-'));
  catalogDir = path.join(tmp, 'catalogs');
  fs.mkdirSync(catalogDir);
});

afterEach(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

describe('translation validator with a Studio type catalog', () => {
  it('accepts a field inherited from a catalog parent that is translatable', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      'archetypes/marketing.yaml': MARKETING_EXTENDS_CONTENT_PAGE,
      [BASE]: 'node_type: studio:Page\narchetype: pkg:MarketingPage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\nmeta_title: Titel\ntagline: Slogan\n',
    };
    expect(notTranslatable(files, load())).toEqual([]);
  });

  it('reports a field inherited from a catalog parent that is not translatable', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      'archetypes/marketing.yaml': MARKETING_EXTENDS_CONTENT_PAGE,
      [BASE]: 'node_type: studio:Page\narchetype: pkg:MarketingPage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\ncanonical_url: https://example.de\n',
    };
    expect(notTranslatable(files, load())).toEqual(['canonical_url']);
  });

  it('closes the chain: a key declared nowhere is reported once the parent is known', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      'archetypes/marketing.yaml': MARKETING_EXTENDS_CONTENT_PAGE,
      [BASE]: 'node_type: studio:Page\narchetype: pkg:MarketingPage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\nstray: nope\n',
    };
    expect(notTranslatable(files, load())).toEqual(['stray']);
  });

  it('resolves an archetype the node names directly from the catalog, through its extends', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      [BASE]: 'node_type: studio:Page\narchetype: standard:ArticlePage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\nbyline: Von\nno_index: true\n',
    };
    expect(notTranslatable(files, load())).toEqual(['no_index']);
  });

  it('checks catalog element types inside a SectionField, including their composites', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      [BASE]: [
        'node_type: studio:Page',
        'archetype: standard:ContentPage',
        'properties:',
        '  content:',
        '    - uuid: h1',
        '      element_type: standard:Hero',
        '      heading: Hi',
        '      variant: dark',
        '      actions: [{ uuid: x1, label: Go, href: /go }]',
      ].join('\n'),
      [DE]: [
        'content:',
        '  - uuid: h1',
        '    heading: Hallo',
        '    variant: light',
        '    actions: [{ uuid: x1, label: Los, href: /los }]',
      ].join('\n'),
    };
    expect(notTranslatable(files, load()).sort()).toEqual(
      ['content[0].actions[0].href', 'content[0].variant'].sort(),
    );
  });

  it('descends into a container element\'s own SectionField', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      [BASE]: [
        'node_type: studio:Page',
        'archetype: standard:ContentPage',
        'properties:',
        '  content:',
        '    - uuid: s1',
        '      element_type: standard:Switcher',
        '      variants:',
        '        - uuid: v1',
        '          element_type: standard:Variant',
        '          label: Evening',
        '          rule: time.hour >= 18',
        '          content:',
        '            - { uuid: h1, element_type: standard:Hero, heading: Hi, variant: dark }',
      ].join('\n'),
      [DE]: [
        'content:',
        '  - uuid: s1',
        '    variants:',
        '      - uuid: v1',
        '        label: Abend',
        '        rule: time.hour >= 19',
        '        content:',
        '          - { uuid: h1, heading: Hallo, variant: light }',
      ].join('\n'),
    };
    expect(notTranslatable(files, load()).sort()).toEqual(
      ['content[0].variants[0].content[0].variant', 'content[0].variants[0].rule'].sort(),
    );
  });

  it('does not report an embedded element (ElementField) as a whole', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      [BASE]: [
        'node_type: studio:Page',
        'archetype: standard:ContentPage',
        'properties:',
        '  content:',
        '    - { uuid: c1, element_type: standard:Card, address: { city: Basel } }',
      ].join('\n'),
      [DE]: 'content:\n  - { uuid: c1, address: { city: Basel } }\n',
    };
    expect(notTranslatable(files, load())).toEqual([]);
  });

  it('lets a package definition win over a catalog definition of the same name', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      // The package ships its own standard:ContentPage where canonical_url IS translatable.
      'archetypes/content-page.yaml': [
        'name: standard:ContentPage',
        'base_node_type: studio:Page',
        'fields:',
        '  - { $type: TextField, name: canonical_url, translatable: true }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\narchetype: standard:ContentPage\nproperties: {}\n',
      [DE]: 'canonical_url: https://example.de\n',
    };
    expect(notTranslatable(files, load())).toEqual([]);
  });

  it('does not use a catalog archetype for the base_node_type fallback', () => {
    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const files = {
      [BASE]: 'node_type: studio:Page\nproperties:\n  title: Hi\n',
      [DE]: 'canonical_url: https://example.de\nstray: nope\n',
    };
    expect(notTranslatable(files, load())).toEqual([]);
  });
});

describe('translation validator without a Studio type catalog', () => {
  it('keeps the skip behavior for keys inherited from an unknown parent', () => {
    const catalogs = load(); // empty directory
    expect(catalogs.catalogs).toEqual([]);
    const files = {
      'archetypes/marketing.yaml': MARKETING_EXTENDS_CONTENT_PAGE,
      [BASE]: 'node_type: studio:Page\narchetype: pkg:MarketingPage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\ncanonical_url: https://example.de\n',
    };
    expect(notTranslatable(files, catalogs)).toEqual([]);
  });

  it('treats a directly named standard:* archetype as unknown', () => {
    const files = {
      [BASE]: 'node_type: studio:Page\narchetype: standard:ArticlePage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\nno_index: true\n',
    };
    expect(notTranslatable(files, load())).toEqual([]);
  });

  it('behaves the same when the catalog directory does not exist', () => {
    const catalogs = loadTypeCatalogs({ catalogDir: path.join(tmp, 'missing'), builtinPaths: NO_BUILTIN });
    expect(catalogs.catalogs).toEqual([]);
    expect(catalogs.ignored).toEqual([]);
    expect(describeTypeCatalogs(catalogs)).toMatch(
      /^No Studio type catalog — inherited fields not checked; run maravilla update/,
    );
  });
});

describe('loading type catalogs', () => {
  it('ignores the assets host SPA fallback (HTML served as 200 for a missing file)', () => {
    writeCatalog('studio-types.json', '<!doctype html><html><body>Studio</body></html>');
    const catalogs = load();
    expect(catalogs.catalogs).toEqual([]);
    expect(catalogs.ignored).toHaveLength(1);
    expect(catalogs.ignored[0].reason).toMatch(/not JSON/);
  });

  it('ignores JSON that is not a v1 raisin-type-catalog', () => {
    writeCatalog('a.json', { format: 'something-else', version: 1, source: 'studio' });
    writeCatalog('b.json', { ...STUDIO_CATALOG, version: 2 });
    writeCatalog('notes.txt', 'not a catalog');
    const catalogs = load();
    expect(catalogs.catalogs).toEqual([]);
    expect(catalogs.ignored.map(i => path.basename(i.file))).toEqual(['a.json', 'b.json']);
  });

  it('picks the highest package_version per source', () => {
    writeCatalog('studio-types-0.3.9.json', { ...STUDIO_CATALOG, package_version: '0.3.9', archetypes: [] });
    writeCatalog('studio-types-0.3.10.json', STUDIO_CATALOG);
    const catalogs = load();
    expect(catalogs.catalogs.map(c => `${c.source} ${c.package_version}`)).toEqual(['studio 0.3.10']);
    expect(catalogs.schemas.archetypes.has('standard:ContentPage')).toBe(true);
  });

  it('loads the builtin catalog and names every catalog in the info line', () => {
    const builtinFile = path.join(tmp, 'builtin-types.json');
    fs.writeFileSync(
      builtinFile,
      JSON.stringify({
        format: 'raisin-type-catalog',
        version: 1,
        source: 'builtin',
        package_version: '0.6.41',
        archetypes: [],
        elementTypes: [],
        nodeTypes: [{ name: 'raisin:Folder', properties: [] }],
      }),
    );
    const onlyBuiltin = loadTypeCatalogs({ catalogDir, builtinPaths: [path.join(tmp, 'nope.json'), builtinFile] });
    expect(describeTypeCatalogs(onlyBuiltin)).toBe(
      'No Studio type catalog — inherited fields not checked; run maravilla update (using builtin 0.6.41)',
    );

    writeCatalog('studio-types.json', STUDIO_CATALOG);
    const both = loadTypeCatalogs({ catalogDir, builtinPaths: [builtinFile] });
    expect(describeTypeCatalogs(both)).toBe('Using type catalogs: studio 0.3.10, builtin 0.6.41');
    expect(both.schemas.nodeTypes.has('raisin:Folder')).toBe(true);
    expect(both.schemas.nodeTypes.has('studio:Page')).toBe(true);
  });

  it('parses catalogs defensively', () => {
    expect(parseTypeCatalog('[]')).toEqual({ error: 'not a JSON object' });
    const c = parseTypeCatalog(
      JSON.stringify({ format: 'raisin-type-catalog', version: 1, source: 'x', archetypes: [{ name: 'a:B' }, { nope: 1 }] }),
    );
    expect('error' in c).toBe(false);
    if (!('error' in c)) {
      expect(c.archetypes).toEqual([{ name: 'a:B', fields: [] }]);
      expect(c.nodeTypes).toEqual([]);
    }
  });

  it('compares versions numerically', () => {
    expect(compareVersions('0.3.10', '0.3.9')).toBeGreaterThan(0);
    expect(compareVersions('0.6.41', '0.6.41')).toBe(0);
    expect(compareVersions('1.0', '1.0.1')).toBeLessThan(0);
  });
});

describe('builtin catalog build script', () => {
  it('reduces YAML type definitions to catalog format v1', () => {
    const nt = path.join(tmp, 'nodetypes');
    const at = path.join(tmp, 'archetypes');
    const et = path.join(tmp, 'elementtypes');
    for (const d of [nt, at, et]) fs.mkdirSync(d);
    fs.writeFileSync(
      path.join(nt, 'folder.yaml'),
      [
        'name: raisin:Folder',
        'description: dropped',
        'version: 3',
        'properties:',
        '  - { name: title, type: String, is_translatable: true, required: true }',
        '  - { name: color, type: String, description: dropped }',
      ].join('\n'),
    );
    fs.writeFileSync(
      path.join(at, 'page.yaml'),
      [
        'name: pkg:Page',
        'base_node_type: raisin:Folder',
        'extends: pkg:Base',
        'meta: { editor: { tabs: [] } }',
        'layouts: []',
        'fields:',
        '  - { $type: TextField, name: title, title: Title, translatable: true, description: dropped }',
        '  - $type: CompositeField',
        '    name: items',
        '    multiple: true',
        '    fields:',
        '      - { $type: TextField, name: label, translatable: true }',
        '      - { $type: TextField, name: href }',
      ].join('\n'),
    );
    fs.writeFileSync(
      path.join(et, 'hero.yaml'),
      'name: pkg:Hero\nfields:\n  - { $type: TextField, name: heading, translatable: false }\n',
    );

    const catalog = buildCatalog({
      nodeTypeDirs: [nt],
      archetypeDirs: [at, path.join(tmp, 'absent')],
      elementTypeDirs: [et],
      source: 'builtin',
      packageVersion: '0.6.41',
    });

    expect(catalog.format).toBe('raisin-type-catalog');
    expect(catalog.version).toBe(1);
    expect(catalog.nodeTypes).toEqual([
      {
        name: 'raisin:Folder',
        properties: [
          { name: 'title', is_translatable: true, type: 'String' },
          { name: 'color', type: 'String' },
        ],
      },
    ]);
    expect(catalog.archetypes).toEqual([
      {
        name: 'pkg:Page',
        base_node_type: 'raisin:Folder',
        extends: 'pkg:Base',
        fields: [
          { $type: 'TextField', name: 'title', translatable: true },
          {
            $type: 'CompositeField',
            name: 'items',
            fields: [
              { $type: 'TextField', name: 'label', translatable: true },
              { $type: 'TextField', name: 'href' },
            ],
          },
        ],
      },
    ]);
    expect(catalog.elementTypes).toEqual([
      { name: 'pkg:Hero', fields: [{ $type: 'TextField', name: 'heading' }] },
    ]);
    // What the script emits is what the loader accepts.
    expect('error' in parseTypeCatalog(JSON.stringify(catalog))).toBe(false);
  });
});
