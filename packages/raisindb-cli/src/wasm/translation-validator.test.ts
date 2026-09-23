import { describe, it, expect } from 'vitest';
import { validateTranslationFiles } from './translation-validator.js';
import { ErrorCodes } from './types.js';

const BASE = 'content/site/page/.node.yaml';
const DE = 'content/site/page/.node.de.yaml';

function notTranslatable(files: Record<string, string>): string[] {
  const results = validateTranslationFiles({ [DE]: files[DE] }, files);
  return results[DE].errors
    .filter(e => e.error_code === ErrorCodes.TRANSLATION_FIELD_NOT_TRANSLATABLE)
    .map(e => e.field_path);
}

describe('translation validator: schema resolution', () => {
  it('treats an explicitly named archetype that is not in the package as unknown', () => {
    const files = {
      // A local archetype with the same base_node_type must NOT be used as a fallback.
      'archetypes/auth-page.yaml': [
        'name: pkg:AuthPage',
        'base_node_type: studio:Page',
        'fields:',
        '  - { $type: TextField, name: heading }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\narchetype: standard:ArticlePage\nproperties:\n  title: Hello\n',
      [DE]: 'title: Hallo\ndescription: Beschreibung\nmeta_title: Titel\n',
    };
    expect(notTranslatable(files)).toEqual([]);
  });

  it('still falls back by base_node_type when the node names no archetype', () => {
    const files = {
      'archetypes/auth-page.yaml': [
        'name: pkg:AuthPage',
        'base_node_type: studio:Page',
        'fields:',
        '  - { $type: TextField, name: heading }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\nproperties:\n  heading: Hello\n',
      [DE]: 'heading: Hallo\n',
    };
    expect(notTranslatable(files)).toEqual(['heading']);
  });

  it('does not report inherited keys when extends reaches a parent outside the package', () => {
    const files = {
      'archetypes/marketing.yaml': [
        'name: pkg:MarketingPage',
        'base_node_type: studio:Page',
        'extends: standard:ContentPage',
        'fields:',
        '  - { $type: SectionField, name: content }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\narchetype: pkg:MarketingPage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\nmeta_title: Titel\n',
    };
    expect(notTranslatable(files)).toEqual([]);
  });

  it('accepts a field the local parent marks translatable', () => {
    const files = {
      'archetypes/base.yaml': [
        'name: pkg:BasePage',
        'fields:',
        '  - { $type: TextField, name: title, translatable: true }',
      ].join('\n'),
      'archetypes/child.yaml': [
        'name: pkg:ChildPage',
        'extends: pkg:BasePage',
        'fields:',
        '  - { $type: TextField, name: subtitle, translatable: true }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\narchetype: pkg:ChildPage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\nsubtitle: Unter\n',
    };
    expect(notTranslatable(files)).toEqual([]);
  });

  it('lets a child override a parent field by name', () => {
    const files = {
      'archetypes/base.yaml': [
        'name: pkg:BasePage',
        'fields:',
        '  - { $type: TextField, name: title, translatable: true }',
      ].join('\n'),
      'archetypes/child.yaml': [
        'name: pkg:ChildPage',
        'extends: pkg:BasePage',
        'fields:',
        '  - { $type: TextField, name: title }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\narchetype: pkg:ChildPage\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\n',
    };
    expect(notTranslatable(files)).toEqual(['title']);
  });

  it('still reports a locally declared non-translatable field on an open chain', () => {
    const files = {
      'archetypes/marketing.yaml': [
        'name: pkg:MarketingPage',
        'extends: standard:ContentPage',
        'fields:',
        '  - { $type: TextField, name: internal_code }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\narchetype: pkg:MarketingPage\nproperties:\n  internal_code: X\n',
      [DE]: 'title: Hallo\ninternal_code: Y\n',
    };
    expect(notTranslatable(files)).toEqual(['internal_code']);
  });

  it('reports unknown keys on a fully local schema (existing behavior)', () => {
    const files = {
      'archetypes/page.yaml': [
        'name: pkg:Page',
        'fields:',
        '  - { $type: TextField, name: title, translatable: true }',
      ].join('\n'),
      [BASE]: 'node_type: studio:Page\narchetype: pkg:Page\nproperties:\n  title: Hi\n',
      [DE]: 'title: Hallo\nstray: nope\n',
    };
    expect(notTranslatable(files)).toEqual(['stray']);
  });

  it('applies the same rules to element types in a SectionField', () => {
    const files = {
      'archetypes/page.yaml': [
        'name: pkg:Page',
        'fields:',
        '  - { $type: SectionField, name: content }',
      ].join('\n'),
      'elementtypes/callout.yaml': [
        'name: pkg:Callout',
        'extends: studio:Component',
        'fields:',
        '  - { $type: TextField, name: body, translatable: true }',
        '  - { $type: TextField, name: variant }',
      ].join('\n'),
      [BASE]: [
        'node_type: studio:Page',
        'archetype: pkg:Page',
        'properties:',
        '  content:',
        '    - { uuid: a1, element_type: pkg:Callout, body: Hi, variant: info }',
      ].join('\n'),
      [DE]: [
        'content:',
        '  - { uuid: a1, body: Hallo, label: inherited, variant: warn }',
      ].join('\n'),
    };
    expect(notTranslatable(files)).toEqual(['content[0].variant']);
  });
});
