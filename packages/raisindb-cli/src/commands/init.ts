import fs from 'fs';
import path from 'path';
import { execSync } from 'child_process';
import { getPack } from '../templates/packs/index.js';
import { writeFileTree } from '../templates/render.js';
import type { TemplateVars } from '../templates/types.js';

interface InitOptions {
  pack?: string;
  name?: string;
  workspace?: string;
  description?: string;
  skipInstall?: boolean;
}

export async function initPackage(folder: string, options: InitOptions): Promise<void> {
  const targetDir = path.resolve(folder);
  const folderName = path.basename(targetDir);

  // Prevent overwriting an existing package
  const manifestPath = path.join(targetDir, 'package', 'manifest.yaml');
  if (fs.existsSync(manifestPath)) {
    throw new Error(`package/manifest.yaml already exists in ${targetDir}. Remove it first or use a different folder.`);
  }

  const packageName = options.name || folderName;
  const workspace = options.workspace || packageName;
  const description = options.description || `A RaisinDB package`;
  const namespace = packageName;

  const vars: TemplateVars = { packageName, workspace, description, namespace };

  const pack = getPack(options.pack || 'minimal');
  const files = pack.getFiles(vars);

  fs.mkdirSync(targetDir, { recursive: true });
  const count = writeFileTree(targetDir, files);

  console.log(`\nInitialized "${packageName}" in ${targetDir}\n`);
  console.log(`  Pack:        ${pack.name}`);
  console.log(`  Workspace:   ${workspace}`);
  console.log(`  Files:       ${count}`);

  // Only the packs that write a root package.json have anything for npm to
  // install or scripts to run; the content-modeling pack ships neither, so
  // `npm install` failed there and "npm run validate" pointed at nothing.
  const hasPackageJson = files.some((f) => f.path === 'package.json');

  if (!options.skipInstall && hasPackageJson) {
    // Run npm install
    console.log(`\nInstalling dependencies...`);
    try {
      execSync('npm install', { cwd: targetDir, stdio: 'inherit' });
    } catch {
      console.warn('  npm install failed — run it manually: cd ' + folder + ' && npm install');
    }

    // Install agent skills
    console.log(`\nInstalling AI agent skills...`);
    try {
      execSync('npx skills add maravilla-labs/raisindb/packages/raisindb-skills', {
        cwd: targetDir,
        stdio: 'inherit',
      });
    } catch {
      console.warn('  Skills install failed — run manually: npx skills add maravilla-labs/raisindb/packages/raisindb-skills');
    }
  }

  console.log(`\nReady! Next steps:`);
  console.log(`  cd ${folder}`);
  if (hasPackageJson) {
    console.log(`  npm run validate`);
  } else {
    console.log(`  raisindb package validate ./package`);
  }
  console.log('');
}
