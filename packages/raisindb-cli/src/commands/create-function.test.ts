import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import yaml from 'yaml';
import { createFunction } from './create-function.js';
import { discoverProjects } from '../wasm-fn/discover.js';
import { registeredHandlers } from '../wasm-fn/handlers.js';
import { runWasmDoctor } from '../wasm-fn/doctor.js';

let root: string;
let log: ReturnType<typeof vi.spyOn>;

const read = (rel: string) => fs.readFileSync(path.join(root, rel), 'utf-8');
const exists = (rel: string) => fs.existsSync(path.join(root, rel));

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-create-fn-'));
  fs.writeFileSync(path.join(root, 'manifest.yaml'), 'name: demo\nversion: 0.1.0\n');
  fs.mkdirSync(path.join(root, 'content'), { recursive: true });
  log = vi.spyOn(console, 'log').mockImplementation(() => undefined);
});

afterEach(() => {
  log.mockRestore();
  fs.rmSync(root, { recursive: true, force: true });
});

describe('create function — scaffold', () => {
  it('splits the node from the toolchain project so sync never sees Cargo.toml', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });

    expect(exists('content/functions/lib/demo/greet/.node.yaml')).toBe(true);
    expect(exists('wasm/demo/greet/Cargo.toml')).toBe(true);
    expect(exists('wasm/demo/greet/src/lib.rs')).toBe(true);
    expect(exists('wasm/demo/greet/raisin.build.yaml')).toBe(true);
    // Source must NOT be under content/: everything non-YAML there is a node.
    expect(exists('content/functions/lib/demo/greet/Cargo.toml')).toBe(false);
    expect(read('.rapignore')).toContain('wasm/');
  });

  it('writes a node whose entry_file selects the scaffolded handler', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });
    const doc = yaml.parse(read('content/functions/lib/demo/greet/.node.yaml'));
    expect(doc.node_type).toBe('raisin:Function');
    expect(doc.properties.language).toBe('wasm');
    expect(doc.properties.entry_file).toBe('main.wasm');

    await createFunction('on-order', { lang: 'rust', ns: 'demo', dir: root, handler: 'on-order' });
    const other = yaml.parse(read('content/functions/lib/demo/on-order/.node.yaml'));
    expect(other.properties.entry_file).toBe('main.wasm:on-order');
  });

  it('points the build file at the node directory it must copy into', async () => {
    await createFunction('greet', { lang: 'go', ns: 'demo', dir: root });
    const spec = yaml.parse(read('wasm/demo/greet/raisin.build.yaml'));
    expect(spec.lang).toBe('go');
    const { projects } = discoverProjects(root);
    expect(projects[0].artifactPath).toBe(
      path.join(root, 'content/functions/lib/demo/greet/main.wasm')
    );
  });

  it('scaffolds a registered handler the doctor recognises', async () => {
    for (const lang of ['rust', 'go', 'ts'] as const) {
      await createFunction(`greet-${lang}`, { lang, ns: 'demo', dir: root });
      const project = discoverProjects(path.join(root, `wasm/demo/greet-${lang}`)).projects[0];
      expect(registeredHandlers(project).names).toEqual(['default']);
    }
    const report = runWasmDoctor(root, { toolchains: false });
    expect(report.findings.filter((f) => f.severity === 'error')).toEqual([]);
  });

  it('refuses to clobber an existing function', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });
    await expect(createFunction('greet', { lang: 'rust', ns: 'demo', dir: root })).rejects.toThrow(
      /already exists/
    );
  });

  it('rejects a name that is not a slug, and an unknown language', async () => {
    await expect(createFunction('Greet Me', { lang: 'rust', dir: root })).rejects.toThrow(
      /lower-kebab-case/
    );
    await expect(createFunction('greet', { lang: 'cobol', dir: root })).rejects.toThrow(/--lang/);
  });

  it('needs a package to scaffold into', async () => {
    const empty = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-nopkg-'));
    await expect(createFunction('greet', { lang: 'rust', dir: empty })).rejects.toThrow(
      /No manifest.yaml/
    );
    fs.rmSync(empty, { recursive: true, force: true });
  });
});

describe('create function --into — one artifact, N functions', () => {
  it('adds a handler to the existing project instead of a second toolchain project', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });
    await createFunction('greet-shout', {
      ns: 'demo',
      dir: root,
      into: 'greet',
      handler: 'shout',
    });

    // No second project, no second component.
    expect(exists('wasm/demo/greet-shout')).toBe(false);
    expect(discoverProjects(root).projects).toHaveLength(1);

    const doc = yaml.parse(read('content/functions/lib/demo/greet-shout/.node.yaml'));
    expect(doc.properties.entry_file).toBe('../greet/main.wasm:shout');

    const project = discoverProjects(root).projects[0];
    expect(registeredHandlers(project).names).toEqual(['default', 'shout']);
    expect(read('wasm/demo/greet/raisin.build.yaml')).toContain('- shout');
    expect(read('wasm/demo/greet/tests/handlers.rs')).toContain('shout');
  });

  it('does the same for go and ts', async () => {
    await createFunction('greet-go', { lang: 'go', ns: 'demo', dir: root });
    await createFunction('shout-go', { ns: 'demo', dir: root, into: 'greet-go', handler: 'shout' });
    const go = discoverProjects(path.join(root, 'wasm/demo/greet-go')).projects[0];
    expect(registeredHandlers(go).names).toEqual(['default', 'shout']);

    await createFunction('greet-ts', { lang: 'ts', ns: 'demo', dir: root });
    await createFunction('shout-ts', { ns: 'demo', dir: root, into: 'greet-ts', handler: 'shout' });
    const ts = discoverProjects(path.join(root, 'wasm/demo/greet-ts')).projects[0];
    expect(registeredHandlers(ts).names).toEqual(['default', 'shout']);
  });

  it('defaults the handler name to the new function name', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });
    await createFunction('shout', { ns: 'demo', dir: root, into: 'greet' });
    const doc = yaml.parse(read('content/functions/lib/demo/shout/.node.yaml'));
    expect(doc.properties.entry_file).toBe('../greet/main.wasm:shout');
  });

  it('refuses a handler name the project already registers', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });
    await expect(
      createFunction('greet-again', { ns: 'demo', dir: root, into: 'greet', handler: 'default' })
    ).rejects.toThrow(/already registers/);
  });

  it('refuses to share an artifact across languages', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });
    await expect(
      createFunction('greet-go', { lang: 'go', ns: 'demo', dir: root, into: 'greet' })
    ).rejects.toThrow(/cannot share its artifact/);
  });

  it('names the projects it knows when --into misses', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: root });
    await expect(
      createFunction('other', { ns: 'demo', dir: root, into: 'nope' })
    ).rejects.toThrow(/Known projects: greet/);
  });
});

describe('create function — a package with no content/ directory yet', () => {
  let bare: string;

  beforeEach(() => {
    bare = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-create-fn-bare-'));
    fs.writeFileSync(path.join(bare, 'manifest.yaml'), 'name: demo\nversion: 0.1.0\n');
  });

  afterEach(() => {
    fs.rmSync(bare, { recursive: true, force: true });
  });

  const bareExists = (rel: string) => fs.existsSync(path.join(bare, rel));

  it('scaffolds into content/, not the package root', async () => {
    // Regression: the scaffold used the READ base, which falls back to the
    // package root when content/ is absent. It wrote functions/lib/... beside
    // the manifest, `package create` packed it, and the installer — which reads
    // only content/{workspace}/... — installed nothing while reporting success.
    await createFunction('greet', { lang: 'rust', ns: 'demo', dir: bare });

    expect(bareExists('content/functions/lib/demo/greet/.node.yaml')).toBe(true);
    expect(bareExists('functions/lib/demo/greet/.node.yaml')).toBe(false);
  });

  it('scaffolds a source-language function into content/ too', async () => {
    await createFunction('hello', { lang: 'js', ns: 'demo', dir: bare });

    expect(bareExists('content/functions/lib/demo/hello/.node.yaml')).toBe(true);
    expect(bareExists('functions/lib/demo/hello/.node.yaml')).toBe(false);
  });
});

describe('create function — --into and the namespace', () => {
  it('puts the sharing node in the target project namespace, not the package name', async () => {
    // The manifest is `demo`; the project lives under the `tools` namespace.
    await createFunction('greet', { lang: 'rust', ns: 'tools', dir: root });
    await createFunction('shout', { into: 'greet', dir: root });

    expect(exists('content/functions/lib/tools/shout/.node.yaml')).toBe(true);
    expect(exists('content/functions/lib/demo/shout/.node.yaml')).toBe(false);

    const doc = yaml.parse(read('content/functions/lib/tools/shout/.node.yaml'));
    expect(doc.properties.entry_file).toBe('../greet/main.wasm:shout');
  });

  it('lets an explicit --ns override the target project namespace', async () => {
    await createFunction('greet', { lang: 'rust', ns: 'tools', dir: root });
    await createFunction('shout', { into: 'greet', ns: 'extra', dir: root });

    expect(exists('content/functions/lib/extra/shout/.node.yaml')).toBe(true);
    const doc = yaml.parse(read('content/functions/lib/extra/shout/.node.yaml'));
    expect(doc.properties.entry_file).toBe('../../tools/greet/main.wasm:shout');
  });
});
