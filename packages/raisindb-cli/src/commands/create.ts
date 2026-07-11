import fs from 'fs';
import path from 'path';
import { adapterFiles, AdapterVars } from '../templates/adapter.js';
import { writeFileTree } from '../templates/render.js';

/** Options for `raisindb create adapter <name>`. */
export interface CreateAdapterOptions {
  /** Target directory. Defaults to `./<name>-adapter`. */
  dir?: string;
  /** Provider type slug for the integration node. Defaults to `<name>`. */
  provider?: string;
  /** One-line package description. */
  description?: string;
}

const SLUG_RE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

/**
 * Scaffold a working connector-adapter package skeleton under a new directory.
 *
 * Emits a manifest (category: integrations, builtin: false), a README, a
 * stub adapter function (`capabilities` + `list` implemented), and a disabled
 * `raisin:Integration` template that carries no client secret. The generated
 * package is immediately installable and passes a `capabilities` invocation
 * with no edits.
 */
export async function createAdapter(
  name: string,
  options: CreateAdapterOptions = {}
): Promise<void> {
  const slug = name.trim().toLowerCase();
  if (!SLUG_RE.test(slug)) {
    throw new Error(
      `Invalid adapter name "${name}". Use lower-kebab-case, e.g. "dropbox" or "box-drive".`
    );
  }

  const targetDir = path.resolve(options.dir || `./${slug}-adapter`);

  // Refuse to clobber an existing scaffold.
  const manifestPath = path.join(targetDir, 'manifest.yaml');
  if (fs.existsSync(manifestPath)) {
    throw new Error(
      `manifest.yaml already exists in ${targetDir}. Remove it first or choose a different --dir.`
    );
  }

  const provider = (options.provider || slug).trim().toLowerCase();
  const description =
    options.description || `Virtual-node connector adapter for ${slug}.`;

  const vars: AdapterVars = { name: slug, provider, description };
  const files = adapterFiles(vars);

  fs.mkdirSync(targetDir, { recursive: true });
  const count = writeFileTree(targetDir, files);

  console.log(`\nScaffolded connector adapter "${slug}" in ${targetDir}\n`);
  console.log(`  Provider:  ${provider}`);
  console.log(`  Function:  /adapters/${slug}`);
  console.log(`  Files:     ${count}`);
  console.log(`\nNext steps:`);
  console.log(`  1. Implement the TODOs in content/functions/adapters/${slug}/index.js`);
  console.log(`  2. Set auth_url / token_url / scopes in the integration template`);
  console.log(`  3. Deploy:  cd ${targetDir} && raisindb package deploy . --repo <repo> --install`);
  console.log('');
}
