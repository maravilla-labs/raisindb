import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  contentBase,
  discoverFunctionNodes,
  discoverProjects,
  findPackageRoot,
  functionsRoot,
  loadProject,
  nodesForArtifact,
  resolveEntryFile,
  splitEntryFile,
} from './discover.js';

let root: string;

/** Write a file, creating parents. */
function write(rel: string, content: string): string {
  const full = path.join(root, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
  return full;
}

function functionNode(name: string, entryFile: string, language = 'wasm'): void {
  write(
    `content/functions/lib/demo/${name}/.node.yaml`,
    `node_type: raisin:Function\nproperties:\n  name: ${name}\n  language: ${language}\n  entry_file: ${entryFile}\n`
  );
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-wasm-'));
  write('manifest.yaml', 'name: demo\nversion: 0.1.0\n');
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe('splitEntryFile', () => {
  it('defaults a bare artifact to the "default" handler', () => {
    expect(splitEntryFile('main.wasm')).toEqual({ asset: 'main.wasm', handler: 'default' });
  });

  it('takes the handler after the colon verbatim', () => {
    expect(splitEntryFile('main.wasm:on-order')).toEqual({
      asset: 'main.wasm',
      handler: 'on-order',
    });
  });

  it('falls back to the default handler on a trailing colon, like the server', () => {
    expect(splitEntryFile('main.wasm:').handler).toBe('default');
  });

  it('keeps a parent-relative asset path intact', () => {
    expect(splitEntryFile('../shared/main.wasm:go')).toEqual({
      asset: '../shared/main.wasm',
      handler: 'go',
    });
  });
});

describe('resolveEntryFile', () => {
  it('resolves a same-directory artifact', () => {
    const nodeDir = path.join(root, 'content/functions/lib/demo/greet');
    const r = resolveEntryFile(nodeDir, 'main.wasm', path.join(root, 'content/functions'));
    expect(r.artifactPath).toBe(path.join(nodeDir, 'main.wasm'));
    expect(r.handler).toBe('default');
    expect(r.escapes).toBe(false);
  });

  it('resolves a parent-relative artifact shared with a sibling node', () => {
    const nodeDir = path.join(root, 'content/functions/lib/demo/greet-shout');
    const r = resolveEntryFile(
      nodeDir,
      '../greet/main.wasm:shout',
      path.join(root, 'content/functions')
    );
    expect(r.artifactPath).toBe(path.join(root, 'content/functions/lib/demo/greet/main.wasm'));
    expect(r.handler).toBe('shout');
    expect(r.escapes).toBe(false);
  });

  it('flags a path that escapes the functions workspace', () => {
    const nodeDir = path.join(root, 'content/functions/lib/demo/greet');
    const r = resolveEntryFile(
      nodeDir,
      '../../../../../etc/passwd.wasm',
      path.join(root, 'content/functions')
    );
    expect(r.escapes).toBe(true);
  });
});

describe('findPackageRoot / contentBase', () => {
  it('walks up to the nearest manifest.yaml', () => {
    write('wasm/demo/greet/raisin.build.yaml', 'lang: rust\nnode_dir: ../..\n');
    expect(findPackageRoot(path.join(root, 'wasm/demo/greet'))).toBe(root);
  });

  it('prefers content/ as the content base when present', () => {
    write('content/functions/.gitkeep', '');
    expect(contentBase(root)).toBe(path.join(root, 'content'));
    expect(functionsRoot(root)).toBe(path.join(root, 'content/functions'));
  });
});

describe('loadProject', () => {
  it('fills in the rust defaults from the crate name', () => {
    const build = write(
      'wasm/demo/greet/raisin.build.yaml',
      'lang: rust\nnode_dir: ../../../content/functions/lib/demo/greet\n'
    );
    write('wasm/demo/greet/Cargo.toml', '[package]\nname = "greet-rust"\n');
    const project = loadProject(build);
    expect(project.command).toContain('--target wasm32-wasip2');
    expect(project.outputPath).toBe(
      path.join(root, 'wasm/demo/greet/target/wasm32-wasip2/release/greet_rust.wasm')
    );
    expect(project.artifactPath).toBe(
      path.join(root, 'content/functions/lib/demo/greet/main.wasm')
    );
  });

  it('rejects an unknown language rather than guessing a toolchain', () => {
    const build = write('wasm/demo/x/raisin.build.yaml', 'lang: cobol\nnode_dir: .\n');
    expect(() => loadProject(build)).toThrow(/lang must be one of/);
  });

  it('reports a build file with no node_dir', () => {
    const build = write('wasm/demo/x/raisin.build.yaml', 'lang: go\n');
    expect(() => loadProject(build)).toThrow(/node_dir is required/);
  });
});

describe('discoverProjects', () => {
  it('finds every project under a package and reports bad ones separately', () => {
    write('wasm/demo/a/raisin.build.yaml', 'lang: rust\nnode_dir: ../../../content/functions/lib/demo/a\n');
    write('wasm/demo/b/raisin.build.yaml', 'lang: nope\nnode_dir: .\n');
    const { projects, failures } = discoverProjects(root);
    expect(projects).toHaveLength(1);
    expect(failures).toHaveLength(1);
  });

  it('does not descend into target/ or node_modules/', () => {
    write('wasm/demo/a/target/raisin.build.yaml', 'lang: rust\nnode_dir: .\n');
    write('wasm/demo/a/node_modules/x/raisin.build.yaml', 'lang: ts\nnode_dir: .\n');
    expect(discoverProjects(root).projects).toHaveLength(0);
  });
});

describe('discoverFunctionNodes', () => {
  it('reads language, entry_file and the handler each node selects', () => {
    functionNode('greet', 'main.wasm');
    functionNode('greet-shout', '../greet/main.wasm:shout');
    functionNode('legacy', 'index.js', 'javascript');

    const nodes = discoverFunctionNodes(root);
    expect(nodes.map((n) => n.name).sort()).toEqual(['greet', 'greet-shout', 'legacy']);

    const shout = nodes.find((n) => n.name === 'greet-shout')!;
    expect(shout.handler).toBe('shout');
    expect(shout.escapes).toBe(false);
    expect(shout.artifactPath).toBe(path.join(root, 'content/functions/lib/demo/greet/main.wasm'));
  });

  it('groups the nodes one artifact backs', () => {
    functionNode('greet', 'main.wasm');
    functionNode('greet-shout', '../greet/main.wasm:shout');
    const nodes = discoverFunctionNodes(root);
    const backs = nodesForArtifact(
      nodes,
      path.join(root, 'content/functions/lib/demo/greet/main.wasm')
    );
    expect(backs.map((n) => n.handler).sort()).toEqual(['default', 'shout']);
  });
});
