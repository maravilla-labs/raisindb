import { describe, it, expect } from 'vitest';
import {
  EnvContext,
  emptyEnvContext,
  formatUnresolvedError,
  hasEnvTokens,
  substituteEnvTokens,
} from './substitute.js';

function env(values: Record<string, string>): EnvContext {
  return { values, sources: ['test'] };
}

describe('hasEnvTokens', () => {
  it('detects the token prefix', () => {
    expect(hasEnvTokens('base_url: "{env:PREVIEW_SERVER}"')).toBe(true);
    expect(hasEnvTokens('base_url: http://localhost:5173')).toBe(false);
  });
});

describe('substituteEnvTokens', () => {
  it('replaces a token with the environment value', () => {
    const { text, unresolved } = substituteEnvTokens(
      'base_url: "{env:PREVIEW_SERVER}"',
      env({ PREVIEW_SERVER: 'https://preview.example.ch' })
    );
    expect(text).toBe('base_url: "https://preview.example.ch"');
    expect(unresolved).toEqual([]);
  });

  it('falls back to an inline default when the variable is unset', () => {
    const { text, unresolved } = substituteEnvTokens(
      'base_url: "{env:PREVIEW_SERVER:-http://localhost:5173}"',
      emptyEnvContext()
    );
    expect(text).toBe('base_url: "http://localhost:5173"');
    expect(unresolved).toEqual([]);
  });

  it('prefers the environment value over the inline default', () => {
    const { text } = substituteEnvTokens(
      '{env:PREVIEW_SERVER:-http://localhost:5173}',
      env({ PREVIEW_SERVER: 'https://prod.example.ch' })
    );
    expect(text).toBe('https://prod.example.ch');
  });

  it('treats an empty environment value as set', () => {
    const { text, unresolved } = substituteEnvTokens('[{env:SUFFIX}]', env({ SUFFIX: '' }));
    expect(text).toBe('[]');
    expect(unresolved).toEqual([]);
  });

  it('supports an empty inline default', () => {
    const { text, unresolved } = substituteEnvTokens('[{env:SUFFIX:-}]', emptyEnvContext());
    expect(text).toBe('[]');
    expect(unresolved).toEqual([]);
  });

  it('reports unresolved tokens with line and column, leaving the text intact', () => {
    const yaml = [
      'properties:',
      '  domain: example.test',
      '  dev_url: "{env:PREVIEW_SERVER}"',
    ].join('\n');

    const { text, unresolved } = substituteEnvTokens(yaml, emptyEnvContext());

    expect(text).toBe(yaml);
    expect(unresolved).toHaveLength(1);
    expect(unresolved[0]).toEqual({
      name: 'PREVIEW_SERVER',
      line: 3,
      column: 13,
      raw: '{env:PREVIEW_SERVER}',
    });
  });

  it('reports every unresolved token on a line', () => {
    const { unresolved } = substituteEnvTokens(
      'url: "{env:HOST}:{env:PORT}"',
      emptyEnvContext()
    );
    expect(unresolved.map((t) => t.name)).toEqual(['HOST', 'PORT']);
  });

  it('resolves several tokens on one line', () => {
    const { text } = substituteEnvTokens(
      'url: "{env:HOST}:{env:PORT}"',
      env({ HOST: 'example.ch', PORT: '8080' })
    );
    expect(text).toBe('url: "example.ch:8080"');
  });

  it('substitutes inside a folded HTML block', () => {
    const yaml = [
      'body: >-',
      '  <a href="{env:SITE_URL}/imprint">Imprint</a>',
    ].join('\n');
    const { text } = substituteEnvTokens(yaml, env({ SITE_URL: 'https://example.ch' }));
    expect(text).toContain('<a href="https://example.ch/imprint">Imprint</a>');
  });

  it('substitutes inside a flow-style mapping', () => {
    const { text } = substituteEnvTokens(
      '- { label: Tickets, href: "{env:TICKET_URL}", external: true }',
      env({ TICKET_URL: 'https://tickets.example.ch' })
    );
    expect(text).toBe(
      '- { label: Tickets, href: "https://tickets.example.ch", external: true }'
    );
  });

  it('leaves flow-engine templates untouched', () => {
    const source = 'to: "{{ trigger.node.properties.email }}" and "${step.out}"';
    const { text, unresolved } = substituteEnvTokens(source, env({ email: 'x' }));
    expect(text).toBe(source);
    expect(unresolved).toEqual([]);
  });

  it('honours a backslash escape and emits the literal token', () => {
    const { text, unresolved } = substituteEnvTokens(
      '# write \\{env:PREVIEW_SERVER} to bind this to the environment',
      env({ PREVIEW_SERVER: 'https://prod.example.ch' })
    );
    expect(text).toBe('# write {env:PREVIEW_SERVER} to bind this to the environment');
    expect(unresolved).toEqual([]);
  });

  it('ignores malformed tokens', () => {
    const source = '{env:} {env:9BAD} {environment}';
    const { text, unresolved } = substituteEnvTokens(source, emptyEnvContext());
    expect(text).toBe(source);
    expect(unresolved).toEqual([]);
  });

  it('returns content unchanged when there is nothing to do', () => {
    const source = 'base_url: http://localhost:5173\n';
    expect(substituteEnvTokens(source, emptyEnvContext()).text).toBe(source);
  });
});

describe('formatUnresolvedError', () => {
  it('lists every occurrence with its location and the sources consulted', () => {
    const { unresolved } = substituteEnvTokens(
      'a: "{env:MISSING}"',
      emptyEnvContext()
    );
    const message = formatUnresolvedError(
      [{ path: 'content/story/.node.yaml', unresolved }],
      { values: {}, sources: ['/pkg/.env', 'process environment'] }
    );

    expect(message).toContain('content/story/.node.yaml:1:5  {env:MISSING}');
    expect(message).toContain('{env:NAME:-fallback}');
    expect(message).toContain('/pkg/.env');
  });
});
